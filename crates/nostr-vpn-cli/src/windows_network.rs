use std::ffi::c_void;
use std::{io, ptr, thread, time::Duration};

use tokio::sync::mpsc;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    CancelMibChangeNotify2, MIB_IPFORWARD_ROW2, MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE,
    MIB_UNICASTIPADDRESS_ROW, NotifyIpInterfaceChange, NotifyRouteChange2,
    NotifyUnicastIpAddressChange,
};
use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;

pub(crate) fn spawn_windows_route_change_monitor() -> Option<mpsc::Receiver<()>> {
    let (tx, rx) = mpsc::channel(1);
    let context = Box::into_raw(Box::new(WindowsNetworkChangeContext { tx }));
    let mut handles = Vec::new();

    if let Some(handle) = register_windows_network_change_callback(context) {
        handles.push(handle);
    }
    if let Some(handle) = register_windows_route_change_callback(context) {
        handles.push(handle);
    }
    if let Some(handle) = register_windows_unicast_address_change_callback(context) {
        handles.push(handle);
    }

    if handles.is_empty() {
        unsafe {
            drop(Box::from_raw(context));
        }
        return None;
    }

    let monitor = WindowsNetworkChangeMonitor { handles, context };
    let spawn_result = thread::Builder::new()
        .name("nvpn-windows-network-monitor".to_string())
        .spawn(move || {
            while !monitor.is_closed() {
                thread::sleep(Duration::from_secs(60));
            }
        });

    match spawn_result {
        Ok(_) => Some(rx),
        Err(error) => {
            eprintln!("daemon: failed to spawn Windows network monitor: {error}");
            None
        }
    }
}

fn register_windows_network_change_callback(
    context: *mut WindowsNetworkChangeContext,
) -> Option<HANDLE> {
    let mut handle = ptr::null_mut();
    let status = unsafe {
        NotifyIpInterfaceChange(
            AF_UNSPEC,
            Some(windows_network_change_callback),
            context.cast::<c_void>(),
            false,
            &mut handle,
        )
    };
    windows_notification_handle(status, handle, "interface")
}

fn register_windows_route_change_callback(
    context: *mut WindowsNetworkChangeContext,
) -> Option<HANDLE> {
    let mut handle = ptr::null_mut();
    let status = unsafe {
        NotifyRouteChange2(
            AF_UNSPEC,
            Some(windows_route_change_callback),
            context.cast::<c_void>(),
            false,
            &mut handle,
        )
    };
    windows_notification_handle(status, handle, "route")
}

fn register_windows_unicast_address_change_callback(
    context: *mut WindowsNetworkChangeContext,
) -> Option<HANDLE> {
    let mut handle = ptr::null_mut();
    let status = unsafe {
        NotifyUnicastIpAddressChange(
            AF_UNSPEC,
            Some(windows_unicast_address_change_callback),
            context.cast::<c_void>(),
            false,
            &mut handle,
        )
    };
    windows_notification_handle(status, handle, "unicast address")
}

fn windows_notification_handle(status: u32, handle: HANDLE, label: &str) -> Option<HANDLE> {
    if status == 0 && !handle.is_null() {
        Some(handle)
    } else {
        eprintln!(
            "daemon: failed to register Windows {label} change callback: {}",
            io::Error::from_raw_os_error(status as i32)
        );
        None
    }
}

unsafe extern "system" fn windows_network_change_callback(
    context: *const c_void,
    _row: *const MIB_IPINTERFACE_ROW,
    _notification_type: MIB_NOTIFICATION_TYPE,
) {
    notify_windows_network_change(context);
}

unsafe extern "system" fn windows_route_change_callback(
    context: *const c_void,
    _row: *const MIB_IPFORWARD_ROW2,
    _notification_type: MIB_NOTIFICATION_TYPE,
) {
    notify_windows_network_change(context);
}

unsafe extern "system" fn windows_unicast_address_change_callback(
    context: *const c_void,
    _row: *const MIB_UNICASTIPADDRESS_ROW,
    _notification_type: MIB_NOTIFICATION_TYPE,
) {
    notify_windows_network_change(context);
}

fn notify_windows_network_change(context: *const c_void) {
    if context.is_null() {
        return;
    }
    let context = unsafe { &*(context.cast::<WindowsNetworkChangeContext>()) };
    match context.tx.try_send(()) {
        Ok(()) | Err(mpsc::error::TrySendError::Full(())) => {}
        Err(mpsc::error::TrySendError::Closed(())) => {}
    }
}

struct WindowsNetworkChangeContext {
    tx: mpsc::Sender<()>,
}

struct WindowsNetworkChangeMonitor {
    handles: Vec<HANDLE>,
    context: *mut WindowsNetworkChangeContext,
}

unsafe impl Send for WindowsNetworkChangeMonitor {}

impl WindowsNetworkChangeMonitor {
    fn is_closed(&self) -> bool {
        unsafe { (*self.context).tx.is_closed() }
    }
}

impl Drop for WindowsNetworkChangeMonitor {
    fn drop(&mut self) {
        for handle in self.handles.drain(..) {
            unsafe {
                CancelMibChangeNotify2(handle);
            }
        }
        unsafe {
            drop(Box::from_raw(self.context));
        }
    }
}
