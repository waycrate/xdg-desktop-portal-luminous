use libwayshot::CaptureRegion;
use libwayshot::{reexport::WlOutput, WayshotConnection};
use pipewire::{
    spa::{
        self,
        pod::{self, deserialize::PodDeserializer, serialize::PodSerializer},
    },
    stream::StreamState,
};
use rustix::fd::BorrowedFd;

use std::{cell::RefCell, io, os::fd::IntoRawFd, rc::Rc, slice};

use tokio::sync::oneshot;

pub struct ScreencastThread {
    node_id: u32,
    thread_stop_tx: pipewire::channel::Sender<()>,
}

impl ScreencastThread {
    pub async fn start_cast(
        overlay_cursor: bool,
        width: u32,
        height: u32,
        capture_region: Option<CaptureRegion>,
        output: WlOutput,
        connection: WayshotConnection,
    ) -> anyhow::Result<Self> {
        let (tx, rx) = oneshot::channel();
        let (thread_stop_tx, thread_stop_rx) = pipewire::channel::channel::<()>();
        std::thread::spawn(move || {
            match start_stream(
                connection,
                overlay_cursor,
                width,
                height,
                capture_region,
                output,
            ) {
                Ok((loop_, listener, context, node_id_rx)) => {
                    tx.send(Ok(node_id_rx)).unwrap();
                    let weak_loop = loop_.downgrade();
                    let _receiver = thread_stop_rx.attach(loop_.loop_(), move |()| {
                        weak_loop.upgrade().unwrap().quit();
                    });
                    loop_.run();
                    // XXX fix segfault with opposite drop order
                    drop(listener);
                    drop(context);
                }
                Err(err) => tx.send(Err(err)).unwrap(),
            };
        });
        Ok(Self {
            node_id: rx.await??.await??,
            thread_stop_tx,
        })
    }

    pub fn node_id(&self) -> u32 {
        self.node_id
    }

    pub fn stop(&self) {
        let _ = self.thread_stop_tx.send(());
    }
}

type PipewireStreamResult = (
    pipewire::main_loop::MainLoop,
    pipewire::stream::StreamListener<()>,
    pipewire::context::Context,
    oneshot::Receiver<anyhow::Result<u32>>,
);

fn start_stream(
    connection: WayshotConnection,
    overlay_cursor: bool,
    width: u32,
    height: u32,
    capture_region: Option<CaptureRegion>,
    output: WlOutput,
) -> Result<PipewireStreamResult, pipewire::Error> {
    let loop_ = pipewire::main_loop::MainLoop::new(None).unwrap();
    let context = pipewire::context::Context::new(&loop_).unwrap();
    let core = context.connect(None).unwrap();

    let name = "wayshot-screenshot"; // XXX randomize?

    let stream = pipewire::stream::Stream::new(
        &core,
        name,
        pipewire::properties::properties! {
            "media.class" => "Video/Source",
            "node.name" => "wayshot-screenshot", // XXX
        },
    )?;

    let (node_id_tx, node_id_rx) = oneshot::channel();
    let mut node_id_tx = Some(node_id_tx);
    let stream_cell: Rc<RefCell<Option<pipewire::stream::Stream>>> = Rc::new(RefCell::new(None));
    let stream_cell_clone = stream_cell.clone();

    let listener = stream
        .add_local_listener_with_user_data(())
        .state_changed(move |_, _, old, new| {
            tracing::info!("state-changed '{:?}' -> '{:?}'", old, new);
            match new {
                StreamState::Paused => {
                    let stream = stream_cell_clone.borrow_mut();
                    let stream = stream.as_ref().unwrap();
                    if let Some(node_id_tx) = node_id_tx.take() {
                        node_id_tx.send(Ok(stream.node_id())).unwrap();
                    }
                }
                StreamState::Error(e) => {
                    tracing::error!("Error! : {e}");
                }
                _ => {}
            }
        })
        .param_changed(|_, _, id, pod| {
            if id != libspa_sys::SPA_PARAM_Format {
                return;
            }
            if let Some(pod) = pod {
                let value = PodDeserializer::deserialize_from::<pod::Value>(pod.as_bytes());
                tracing::info!("param-changed: {} {:?}", id, value);
            }
        })
        .add_buffer(move |_, _, buffer| {
            let buf = unsafe { &mut *(*buffer).buffer };
            let datas = unsafe { slice::from_raw_parts_mut(buf.datas, buf.n_datas as usize) };
            for data in datas {
                let name = c"pipewire-screencopy";
                let fd = rustix::fs::memfd_create(name, rustix::fs::MemfdFlags::CLOEXEC).unwrap();
                rustix::fs::ftruncate(&fd, (width * height * 4) as _).unwrap();

                data.type_ = libspa_sys::SPA_DATA_MemFd;
                data.flags = 0;
                data.fd = fd.into_raw_fd().into();

                data.data = std::ptr::null_mut();
                data.maxsize = width * height * 4;
                data.mapoffset = 0;
                let chunk = unsafe { &mut *data.chunk };
                chunk.size = width * height * 4;
                chunk.offset = 0;
                chunk.stride = 4 * width as i32;
            }
        })
        .remove_buffer(|_, _, buffer| {
            let buf = unsafe { &mut *(*buffer).buffer };
            let datas = unsafe { slice::from_raw_parts_mut(buf.datas, buf.n_datas as usize) };

            for data in datas {
                unsafe { rustix::io::close(data.fd as _) };
                data.fd = -1;
            }
        })
        .process(move |stream, ()| {
            if let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                let fd = unsafe { BorrowedFd::borrow_raw(datas[0].as_raw().fd as _) };
                // TODO error
                connection
                    .capture_output_frame_shm_fd(overlay_cursor as i32, &output, fd, capture_region)
                    .unwrap();
            }
        })
        .register()?;

    let format = format(width, height);
    let buffers = buffers(width, height);

    let params = &mut [
        pod::Pod::from_bytes(&format).unwrap(),
        pod::Pod::from_bytes(&buffers).unwrap(),
    ];

    let flags = pipewire::stream::StreamFlags::ALLOC_BUFFERS;
    stream.connect(pipewire::spa::utils::Direction::Output, None, flags, params)?;

    *stream_cell.borrow_mut() = Some(stream);

    Ok((loop_, listener, context, node_id_rx))
}

fn value_to_bytes(value: pod::Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut cursor = io::Cursor::new(&mut bytes);
    PodSerializer::serialize(&mut cursor, &value).unwrap();
    bytes
}

fn buffers(width: u32, height: u32) -> Vec<u8> {
    value_to_bytes(pod::Value::Object(pod::Object {
        type_: libspa_sys::SPA_TYPE_OBJECT_ParamBuffers,
        id: libspa_sys::SPA_PARAM_Buffers,
        properties: vec![
            /*
            pod::Property {
                key: spa_sys::SPA_PARAM_BUFFERS_dataType,
                flags: pod::PropertyFlags::empty(),
                value: pod::Value::Choice(pod::ChoiceValue::Int(spa::utils::Choice(
                    spa::utils::ChoiceFlags::empty(),
                    spa::utils::ChoiceEnum::Flags {
                        default: 1 << spa_sys::SPA_DATA_MemFd,
                        flags: vec![],
                    },
                ))),
            },
            */
            pod::Property {
                key: libspa_sys::SPA_PARAM_BUFFERS_size,
                flags: pod::PropertyFlags::empty(),
                value: pod::Value::Int(width as i32 * height as i32 * 4),
            },
            pod::Property {
                key: libspa_sys::SPA_PARAM_BUFFERS_stride,
                flags: pod::PropertyFlags::empty(),
                value: pod::Value::Int(width as i32 * 4),
            },
            pod::Property {
                key: libspa_sys::SPA_PARAM_BUFFERS_align,
                flags: pod::PropertyFlags::empty(),
                value: pod::Value::Int(16),
            },
            pod::Property {
                key: libspa_sys::SPA_PARAM_BUFFERS_blocks,
                flags: pod::PropertyFlags::empty(),
                value: pod::Value::Int(1),
            },
            pod::Property {
                key: libspa_sys::SPA_PARAM_BUFFERS_buffers,
                flags: pod::PropertyFlags::empty(),
                value: pod::Value::Choice(pod::ChoiceValue::Int(spa::utils::Choice(
                    spa::utils::ChoiceFlags::empty(),
                    spa::utils::ChoiceEnum::Range {
                        default: 4,
                        min: 1,
                        max: 32,
                    },
                ))),
            },
        ],
    }))
}

#[allow(unused)]
fn buffers2(width: u32, height: u32) -> Vec<u8> {
    value_to_bytes(pod::Value::Object(spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamBuffers,
        spa::param::ParamType::Buffers,
    )))
}

fn format(width: u32, height: u32) -> Vec<u8> {
    value_to_bytes(pod::Value::Object(spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            spa::param::format::MediaType::Video
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            spa::param::format::MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::RGBA,
            spa::param::video::VideoFormat::RGBA,
        ),
        // XXX modifiers
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle { width, height },
            spa::utils::Rectangle { width, height },
            spa::utils::Rectangle { width, height }
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction { num: 60, denom: 1 },
            spa::utils::Fraction { num: 60, denom: 1 },
            spa::utils::Fraction { num: 60, denom: 1 }
        ),
        // TODO max framerate
    )))
}
