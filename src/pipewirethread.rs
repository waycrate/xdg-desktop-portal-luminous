use libwayshot::region::EmbeddedRegion;
use libwayshot::{WayshotConnection, reexport::WlOutput};
use pipewire::{
    spa::{
        self,
        param::video::{VideoFormat, VideoInfoRaw},
        pod::{self, serialize::PodSerializer},
    },
    stream::StreamState,
};
use rustix::fd::BorrowedFd;
use std::{cell::RefCell, io, os::fd::IntoRawFd, rc::Rc, slice};
use wayland_client::protocol::wl_shm::Format;

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
        embedded_region: Option<EmbeddedRegion>,
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
                embedded_region,
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
    pipewire::stream::StreamListener<Option<Format>>,
    pipewire::context::Context,
    oneshot::Receiver<anyhow::Result<u32>>,
);

fn start_stream(
    connection: WayshotConnection,
    overlay_cursor: bool,
    width: u32,
    height: u32,
    embedded_region: Option<EmbeddedRegion>,
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

    let available_video_formats = match connection.get_available_frame_formats(&output) {
        Ok(frame_format_list) => frame_format_list
            .iter()
            .filter_map(|frame_format| wl_shm_format_to_spa(frame_format.format))
            .collect(),
        Err(e) => {
            tracing::warn!("Could not get available video formats from libwayshot: {e}");
            // Xrgb8888 and Argb8888 should be supported by all renderers
            // https://smithay.github.io/wayland-rs/wayland_client/protocol/wl_shm/enum.Format.html
            vec![VideoFormat::BGRx, VideoFormat::BGRA]
        }
    };
    let chosen_format: Option<Format> = None;

    let listener = stream
        .add_local_listener_with_user_data(chosen_format)
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
        .param_changed(|_, chosen_format, id, pod| {
            if id != libspa_sys::SPA_PARAM_Format {
                return;
            }
            if let Some(pod) = pod {
                let mut chosen_format_info = VideoInfoRaw::new();
                match chosen_format_info.parse(pod) {
                    Ok(_) => {
                        *chosen_format =
                            Some(spa_format_to_wl_shm(chosen_format_info.format()).unwrap())
                    }
                    Err(e) => {
                        tracing::error!("Could not parse format chosen by PipeWire server: {e}")
                    }
                };
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
        .process(move |stream, chosen_format| {
            if let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                let fd = unsafe { BorrowedFd::borrow_raw(datas[0].as_raw().fd as _) };
                match chosen_format {
                    Some(format) => {
                        // TODO error
                        connection
                            .capture_output_frame_shm_fd_with_format(
                                overlay_cursor as i32,
                                &output,
                                fd,
                                *format,
                                embedded_region,
                            )
                            .unwrap();
                    }
                    None => {
                        tracing::error!(
                            "Pipewire: couldn't capture video frames, chosen format is empty"
                        );
                    }
                }
            }
        })
        .register()?;

    let format = format(width, height, available_video_formats);
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

fn format(width: u32, height: u32, available_video_formats: Vec<VideoFormat>) -> Vec<u8> {
    let mut obj = spa::pod::object!(
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
    );

    let format_choice =
        pod::Value::Choice(pod::ChoiceValue::Id(spa::utils::Choice::<spa::utils::Id>(
            spa::utils::ChoiceFlags::empty(),
            spa::utils::ChoiceEnum::<spa::utils::Id>::Enum {
                default: spa::utils::Id(VideoFormat::BGRA.as_raw()),
                alternatives: available_video_formats
                    .iter()
                    .map(|f| spa::utils::Id(f.as_raw()))
                    .collect(),
            },
        )));

    obj.properties.push(pod::Property {
        key: spa::param::format::FormatProperties::VideoFormat.as_raw(),
        flags: pod::PropertyFlags::empty(),
        value: format_choice,
    });
    value_to_bytes(pod::Value::Object(obj))
}

// wl_shm::Format uses FourCC codes, hence the conversion logic
fn spa_format_to_wl_shm(format: VideoFormat) -> Option<Format> {
    match format {
        VideoFormat::BGRA => Some(Format::Argb8888),
        VideoFormat::BGRx => Some(Format::Xrgb8888),
        VideoFormat::ABGR => Some(Format::Rgba8888),
        VideoFormat::xBGR => Some(Format::Rgbx8888),
        VideoFormat::RGBA => Some(Format::Abgr8888),
        VideoFormat::RGBx => Some(Format::Xbgr8888),
        VideoFormat::ARGB => Some(Format::Bgra8888),
        VideoFormat::xRGB => Some(Format::Bgrx8888),
        VideoFormat::xRGB_210LE => Some(Format::Xrgb2101010),
        VideoFormat::xBGR_210LE => Some(Format::Xbgr2101010),
        VideoFormat::RGBx_102LE => Some(Format::Rgbx1010102),
        VideoFormat::BGRx_102LE => Some(Format::Bgrx1010102),
        VideoFormat::ARGB_210LE => Some(Format::Argb2101010),
        VideoFormat::ABGR_210LE => Some(Format::Abgr2101010),
        VideoFormat::RGBA_102LE => Some(Format::Rgba1010102),
        VideoFormat::BGRA_102LE => Some(Format::Bgra1010102),
        _ => None,
    }
}

fn wl_shm_format_to_spa(format: Format) -> Option<VideoFormat> {
    match format {
        Format::Argb8888 => Some(VideoFormat::BGRA),
        Format::Xrgb8888 => Some(VideoFormat::BGRx),
        Format::Rgba8888 => Some(VideoFormat::ABGR),
        Format::Rgbx8888 => Some(VideoFormat::xBGR),
        Format::Abgr8888 => Some(VideoFormat::RGBA),
        Format::Xbgr8888 => Some(VideoFormat::RGBx),
        Format::Bgra8888 => Some(VideoFormat::ARGB),
        Format::Bgrx8888 => Some(VideoFormat::xRGB),
        Format::Xrgb2101010 => Some(VideoFormat::xRGB_210LE),
        Format::Xbgr2101010 => Some(VideoFormat::xBGR_210LE),
        Format::Rgbx1010102 => Some(VideoFormat::RGBx_102LE),
        Format::Bgrx1010102 => Some(VideoFormat::BGRx_102LE),
        Format::Argb2101010 => Some(VideoFormat::ARGB_210LE),
        Format::Abgr2101010 => Some(VideoFormat::ABGR_210LE),
        Format::Rgba1010102 => Some(VideoFormat::RGBA_102LE),
        Format::Bgra1010102 => Some(VideoFormat::BGRA_102LE),
        _ => None,
    }
}
