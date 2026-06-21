use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1,
    ext_data_control_manager_v1::{self, ExtDataControlManagerV1},
    ext_data_control_offer_v1, ext_data_control_source_v1,
};

use wayland_client::{Dispatch, delegate_noop, event_created_child, protocol::wl_seat::WlSeat};

pub struct ClipboardWl;

impl Dispatch<ext_data_control_device_v1::ExtDataControlDeviceV1, ()> for ClipboardWl {
    fn event(
        state: &mut Self,
        proxy: &ext_data_control_device_v1::ExtDataControlDeviceV1,
        event: <ext_data_control_device_v1::ExtDataControlDeviceV1 as wayland_client::Proxy>::Event,
        data: &(),
        conn: &wayland_client::Connection,
        qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
    event_created_child!(ClipboardWl, ext_data_control_device_v1::ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ext_data_control_offer_v1::ExtDataControlOfferV1, ())
    ]);
}

impl Dispatch<ext_data_control_source_v1::ExtDataControlSourceV1, ()> for ClipboardWl {
    fn event(
        state: &mut Self,
        proxy: &ext_data_control_source_v1::ExtDataControlSourceV1,
        event: <ext_data_control_source_v1::ExtDataControlSourceV1 as wayland_client::Proxy>::Event,
        data: &(),
        conn: &wayland_client::Connection,
        qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_data_control_offer_v1::ExtDataControlOfferV1, ()> for ClipboardWl {
    fn event(
        state: &mut Self,
        proxy: &ext_data_control_offer_v1::ExtDataControlOfferV1,
        event: <ext_data_control_offer_v1::ExtDataControlOfferV1 as wayland_client::Proxy>::Event,
        data: &(),
        conn: &wayland_client::Connection,
        qhandle: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(ClipboardWl: ignore WlSeat);
delegate_noop!(ClipboardWl: ignore ExtDataControlManagerV1);
