use libwayshot::region::EmbeddedRegion;
use libwayshot::{TopLevel, WayshotConnection, WayshotTarget, reexport::WlOutput};
use pipewire::spa::pod::Pod;
use pipewire::{
    spa::{
        self,
        param::video::{VideoFormat, VideoInfoRaw},
        pod::{self, serialize::PodSerializer},
    },
    stream::StreamState,
};
use rustix::fd::BorrowedFd;
use std::{io, os::fd::IntoRawFd, slice};
use wayland_client::WEnum;
use wayland_client::protocol::wl_shm::Format;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::FailureReason;

use tokio::sync::oneshot;

pub struct ScreencastThread {
    node_id: u32,
    thread_stop_tx: pipewire::channel::Sender<()>,
}

#[derive(Debug, Clone)]
pub enum CastTarget {
    TopLevel(TopLevel),
    Screen(WlOutput),
}

impl From<&CastTarget> for WayshotTarget {
    fn from(value: &CastTarget) -> Self {
        match value {
            CastTarget::Screen(screen) => Self::Screen(screen.clone()),
            CastTarget::TopLevel(toplevel) => Self::Toplevel(toplevel.clone()),
        }
    }
}

impl ScreencastThread {
    pub async fn start_cast(
        overlay_cursor: bool,
        embedded_region: Option<EmbeddedRegion>,
        target: CastTarget,
        connection: WayshotConnection,
    ) -> anyhow::Result<Self> {
        let (tx, rx) = oneshot::channel();
        let (thread_stop_tx, thread_stop_rx) = pipewire::channel::channel::<()>();
        std::thread::spawn(move || {
            match start_stream(connection, overlay_cursor, embedded_region, target) {
                Ok((loop_, listener, _stream, context, node_id_rx)) => {
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

#[derive(Debug)]
struct StreamingData {
    chosen_format: Option<Format>,
    connection: WayshotConnection,
    overlay_cursor: bool,
    available_video_formats: Vec<VideoFormat>,
    embedded_region: Option<EmbeddedRegion>,
    size: libwayshot::Size,
    target: CastTarget,
}

impl StreamingData {
    fn new(
        width: u32,
        height: u32,
        connection: WayshotConnection,
        overlay_cursor: bool,
        available_video_formats: Vec<VideoFormat>,
        embedded_region: Option<EmbeddedRegion>,
        target: CastTarget,
    ) -> Self {
        Self {
            chosen_format: None,
            connection,
            overlay_cursor,
            available_video_formats,
            size: libwayshot::Size { width, height },
            embedded_region,
            target,
        }
    }

    fn process(&mut self, stream: &pipewire::stream::StreamRef) {
        let Some(mut buffer) = stream.dequeue_buffer() else {
            return;
        };
        let datas = buffer.datas_mut();
        let fd = unsafe { BorrowedFd::borrow_raw(datas[0].as_raw().fd as _) };
        let Some(chosen_format) = self.chosen_format else {
            tracing::error!("Could not capture video frame, chosen format is empty");
            return;
        };
        let guard_result = match &self.target {
            CastTarget::Screen(output) => self.connection.capture_output_frame_shm_fd_with_format(
                self.overlay_cursor as i32,
                output,
                fd,
                chosen_format,
                self.embedded_region,
            ),
            CastTarget::TopLevel(top_level) => {
                self.connection.capture_toplevel_frame_shm_fd_with_format(
                    self.overlay_cursor,
                    top_level,
                    fd,
                    chosen_format,
                )
            }
        };
        let _guard = match guard_result {
            Ok(guard) => guard,
            Err(libwayshot::Error::FramecopyFailedWithReason(WEnum::Value(
                FailureReason::BufferConstraints,
            ))) => {
                let Ok(size) = try_cast(&self.target, &self.connection) else {
                    return;
                };

                let libwayshot::Size { width, height } = size;
                let format = format(width, height, self.available_video_formats.clone());
                let buffers = buffers(width, height);

                let params = &mut [
                    pod::Pod::from_bytes(&format).unwrap(),
                    pod::Pod::from_bytes(&buffers).unwrap(),
                ];
                if let Err(err) = stream.update_params(params) {
                    tracing::error!("failed to update pipewire params: {}", err);
                }
                self.size = size;
                return;
            }
            Err(e) => {
                tracing::error!("Could not capture video frame: {e}");
                return;
            }
        };
    }
    fn add_buffer(&self, buffer: *mut pipewire::sys::pw_buffer) {
        let libwayshot::Size { width, height } = self.size;
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
    }
    fn remove_buffer(&self, buffer: *mut pipewire::sys::pw_buffer) {
        let buf = unsafe { &mut *(*buffer).buffer };
        let datas = unsafe { slice::from_raw_parts_mut(buf.datas, buf.n_datas as usize) };

        for data in datas {
            unsafe { rustix::io::close(data.fd as _) };
            data.fd = -1;
        }
    }
    fn param_changed(&mut self, id: u32, pod: Option<&Pod>) {
        if id != libspa_sys::SPA_PARAM_Format {
            return;
        }
        if let Some(pod) = pod {
            let mut chosen_format_info = VideoInfoRaw::new();
            match chosen_format_info.parse(pod) {
                Ok(_) => {
                    if let Some(wl_shm_fmt) = spa_format_to_wl_shm(chosen_format_info.format()) {
                        self.chosen_format = Some(wl_shm_fmt);
                    } else {
                        tracing::error!(
                            "Could not convert SPA format chosen by PipeWire server to wl_shm format"
                        );
                    }
                }
                Err(e) => tracing::error!("Could not parse format chosen by PipeWire server: {e}"),
            };
        }
    }
}

type PipewireStreamResult = (
    pipewire::main_loop::MainLoop,
    pipewire::stream::StreamListener<StreamingData>,
    pipewire::stream::Stream,
    pipewire::context::Context,
    oneshot::Receiver<anyhow::Result<u32>>,
);
use std::{
    ffi::CString,
    os::fd::OwnedFd,
    time::{SystemTime, UNIX_EPOCH},
};

fn get_mem_file_handle() -> String {
    format!(
        "/luminous-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|time| time.subsec_nanos().to_string())
            .unwrap_or("unknown".into())
    )
}

pub fn create_shm_fd() -> std::io::Result<OwnedFd> {
    use rustix::{
        fs::{self, SealFlags},
        io, shm,
    };
    // Only try memfd on linux and freebsd.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    loop {
        // Create a file that closes on successful execution and seal it's operations.
        match fs::memfd_create(
            CString::new("luminous")?.as_c_str(),
            fs::MemfdFlags::CLOEXEC | fs::MemfdFlags::ALLOW_SEALING,
        ) {
            Ok(fd) => {
                // This is only an optimization, so ignore errors.
                // F_SEAL_SRHINK = File cannot be reduced in size.
                // F_SEAL_SEAL = Prevent further calls to fcntl().
                let _ = fs::fcntl_add_seals(&fd, fs::SealFlags::SHRINK | SealFlags::SEAL);
                return Ok(fd);
            }
            Err(io::Errno::INTR) => continue,
            Err(io::Errno::NOSYS) => break,
            Err(errno) => return Err(std::io::Error::from(errno)),
        }
    }

    // Fallback to using shm_open.
    let mut mem_file_handle = get_mem_file_handle();
    loop {
        let open_result = shm::open(
            mem_file_handle.as_str(),
            shm::OFlags::CREATE | shm::OFlags::EXCL | shm::OFlags::RDWR,
            fs::Mode::RUSR | fs::Mode::WUSR,
        );
        // O_CREAT = Create file if does not exist.
        // O_EXCL = Error if create and file exists.
        // O_RDWR = Open for reading and writing.
        // O_CLOEXEC = Close on successful execution.
        // S_IRUSR = Set user read permission bit .
        // S_IWUSR = Set user write permission bit.
        match open_result {
            Ok(fd) => match shm::unlink(mem_file_handle.as_str()) {
                Ok(_) => return Ok(fd),
                Err(errno) => return Err(std::io::Error::from(errno)),
            },
            Err(io::Errno::EXIST) => {
                // If a file with that handle exists then change the handle
                mem_file_handle = get_mem_file_handle();
                continue;
            }
            Err(io::Errno::INTR) => continue,
            Err(errno) => return Err(std::io::Error::from(errno)),
        }
    }
}

pub fn try_cast(
    target: &CastTarget,
    connection: &WayshotConnection,
) -> anyhow::Result<libwayshot::Size> {
    let shm_file = create_shm_fd()?;

    let (_, guard_test) = match &target {
        CastTarget::Screen(output) => {
            connection.capture_output_frame_shm_fd(0, output, shm_file, None)?
        }
        CastTarget::TopLevel(toplevel) => {
            connection.capture_toplevel_frame_shm_fd(true, toplevel, shm_file)?
        }
    };

    Ok(guard_test.size)
}

fn start_stream(
    connection: WayshotConnection,
    overlay_cursor: bool,
    embedded_region: Option<EmbeddedRegion>,
    target: CastTarget,
) -> anyhow::Result<PipewireStreamResult> {
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

    let available_video_formats = match connection.get_available_frame_formats(&(&target).into()) {
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
    let libwayshot::Size { width, height } = try_cast(&target, &connection)?;

    let listener = stream
        .add_local_listener_with_user_data(StreamingData::new(
            width,
            height,
            connection,
            overlay_cursor,
            available_video_formats.clone(),
            embedded_region,
            target,
        ))
        .state_changed(move |stream, _, old, new| {
            tracing::info!("state-changed '{:?}' -> '{:?}'", old, new);
            match new {
                StreamState::Paused => {
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
        .param_changed(|_stream, streaming_data, id, pod| {
            streaming_data.param_changed(id, pod);
        })
        .add_buffer(|_, data, buffer| {
            data.add_buffer(buffer);
        })
        .remove_buffer(|_, data, buffer| {
            data.remove_buffer(buffer);
        })
        .process(move |stream, streaming_data| {
            streaming_data.process(stream);
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
    Ok((loop_, listener, stream, context, node_id_rx))
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
