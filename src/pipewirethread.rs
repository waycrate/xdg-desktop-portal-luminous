use libwayshot::CaptureRegion;
use pipewire::{
    spa::{
        self,
        pod::{self, serialize::PodSerializer},
    },
    stream::StreamState,
};
use std::sync::mpsc;
use std::{cell::RefCell, io, os::fd::IntoRawFd, rc::Rc, slice};
use libwayshot::reexport::WlOutput;

#[allow(unused)]
pub struct ScreencastThread {
    node_id: u32,
    thread_stop_tx: pipewire::channel::Sender<()>,
}

impl ScreencastThread {
    pub fn start_cast(
        overlay_cursor: bool,
        width: u32,
        height: u32,
        capture_region: Option<CaptureRegion>,
        output: WlOutput,
    ) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let (thread_stop_tx, thread_stop_rx) = pipewire::channel::channel::<()>();
        std::thread::spawn(move || {
            let _ = start_stream(
                thread_stop_rx,
                tx,
                overlay_cursor,
                width,
                height,
                capture_region,
                output,
            );
        });
        Ok(Self {
            node_id: rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .map_err(|_| anyhow::anyhow!("Timeout"))?,
            thread_stop_tx,
        })
    }

    pub fn node_id(&self) -> u32 {
        self.node_id
    }

    #[allow(unused)]
    pub fn stop(&self) {
        let _ = self.thread_stop_tx.send(());
    }
}
fn start_stream(
    stop_rx: pipewire::channel::Receiver<()>,
    sender: mpsc::Sender<u32>,
    overlay_cursor: bool,
    width: u32,
    height: u32,
    capture_region: Option<CaptureRegion>,
    output: WlOutput,
) -> Result<(), pipewire::Error> {
    let connection = libwayshot::WayshotConnection::new().unwrap();

    let loop_ = pipewire::MainLoop::new()?;
    let context = pipewire::Context::new(&loop_).unwrap();
    let core = context.connect(None).unwrap();

    let name = "wayshot-screenshot"; // XXX randomize?

    let stream = pipewire::stream::Stream::new(
        &core,
        name,
        pipewire::properties! {
            "media.class" => "Video/Source",
            "node.name" => "wayshot-screenshot", // XXX
        },
    )?;

    let mut hassend = false;
    let stream_cell: Rc<RefCell<Option<pipewire::stream::Stream>>> = Rc::new(RefCell::new(None));
    let stream_cell_clone = stream_cell.clone();

    let _listener = stream
        .add_local_listener_with_user_data(())
        .state_changed(move |old, new| {
            println!("state-changed '{:?}' -> '{:?}'", old, new);
            match new {
                StreamState::Streaming => {
                    println!("Streaming");
                }
                StreamState::Paused => {
                    let stream = stream_cell_clone.borrow_mut();
                    let stream = stream.as_ref().unwrap();
                    if !hassend {
                        println!("Send");
                        let _ = sender.send(stream.node_id());
                        hassend = true;
                    }

                    println!("Paused");
                }
                StreamState::Error(_) => {
                    println!("Errror");
                }
                _ => {}
            }
        })
        .param_changed(|_, id, (), pod| {
            if id != libspa_sys::SPA_PARAM_Format {
                return;
            }
            if let Some(pod) = pod {
                println!("param-changed: {} {:?}", id, pod.size());
            }
        })
        .add_buffer(move |buffer| {
            let buf = unsafe { &mut *(*buffer).buffer };
            let datas = unsafe { slice::from_raw_parts_mut(buf.datas, buf.n_datas as usize) };
            for data in datas {
                use std::ffi::CStr;
                let name = unsafe { CStr::from_bytes_with_nul_unchecked(b"pipewire-screencopy\0") };
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
        .remove_buffer(|buffer| {
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
                let fd = datas[0].as_raw().fd as i32;
                // TODO error
                connection.capture_output_frame_shm_fd(
                    overlay_cursor as i32,
                    &output,
                    fd,
                    capture_region,
                ).unwrap();
            }
        })
        .register()?;
    let format = format(width, height);
    let buffers = buffers(width, height);

    let params = &mut [
        //pod::Pod::from_bytes(&values).unwrap(),
        pod::Pod::from_bytes(&format).unwrap(),
        pod::Pod::from_bytes(&buffers).unwrap(),
    ];
    //let flags = pipewire::stream::StreamFlags::MAP_BUFFERS;
    let flags = pipewire::stream::StreamFlags::ALLOC_BUFFERS;
    stream.connect(spa::Direction::Output, None, flags, params)?;

    *stream_cell.borrow_mut() = Some(stream);
    let weak_loop = loop_.downgrade();
    let _receiver = stop_rx.attach(&loop_, move |_| {
        weak_loop.upgrade().unwrap().quit();
    });
    loop_.run();

    Ok(())
    //Ok((loop_, node_id))
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
            spa::format::FormatProperties::MediaType,
            Id,
            spa::format::MediaType::Video
        ),
        spa::pod::property!(
            spa::format::FormatProperties::MediaSubtype,
            Id,
            spa::format::MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::RGBA,
            spa::param::video::VideoFormat::RGBA,
        ),
        // XXX modifiers
        spa::pod::property!(
            spa::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle { width, height },
            spa::utils::Rectangle { width, height },
            spa::utils::Rectangle { width, height }
        ),
        spa::pod::property!(
            spa::format::FormatProperties::VideoFramerate,
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
