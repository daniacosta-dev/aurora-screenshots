//! X11 input grabbing for the capture overlay.

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as RandrConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

fn connect() -> Result<RustConnection, String> {
    RustConnection::connect(None)
        .map(|(conn, _)| conn)
        .map_err(|e| format!("X11 connect: {e}"))
}

/// Returns `(x, y, width, height)` of the primary monitor via RandR, or `None`.
/// More reliable than GDK's `primary_monitor()` which returns `None` on many setups.
pub fn get_primary_monitor_geometry() -> Option<(i32, i32, i32, i32)> {
    let (conn, screen_num) =
        RustConnection::connect(None).map_err(|e| format!("{e}")).ok()?;
    let root = conn.setup().roots[screen_num].root;
    let output = conn.randr_get_output_primary(root).ok()?.reply().ok()?.output;
    if output == 0 {
        eprintln!("[x11_grab] randr: no primary output set");
        return None;
    }
    let info = conn.randr_get_output_info(output, 0).ok()?.reply().ok()?;
    if info.crtc == 0 {
        eprintln!("[x11_grab] randr: primary output has no active CRTC");
        return None;
    }
    let crtc = conn.randr_get_crtc_info(info.crtc, 0).ok()?.reply().ok()?;
    eprintln!("[x11_grab] randr primary: ({},{}) {}x{}", crtc.x, crtc.y, crtc.width, crtc.height);
    Some((crtc.x as i32, crtc.y as i32, crtc.width as i32, crtc.height as i32))
}

/// Sets `override_redirect=1` on an unmapped window before its first `show()`.
/// Must be called BEFORE Tauri maps the window — setting it post-map requires
/// unmap/remap which breaks GTK's event loop.
pub fn set_override_redirect(xid: u32) {
    eprintln!("[x11_grab] set_override_redirect xid={xid}");
    match connect() {
        Err(e) => eprintln!("[x11_grab] set_override_redirect: connect failed: {e}"),
        Ok(conn) => {
            let _ = conn.change_window_attributes(
                xid,
                &ChangeWindowAttributesAux::new().override_redirect(1),
            );
            let _ = conn.flush();
            eprintln!("[x11_grab] override_redirect=1 OK");
        }
    }
}

/// Raises the overlay, sets keyboard focus, grabs pointer and keyboard.
pub fn setup_and_grab(xid: u32) -> Result<(), String> {
    eprintln!("[x11_grab] setup_and_grab START xid={xid}");
    let conn = connect()?;

    let _ = conn.configure_window(xid, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
    let _ = conn.set_input_focus(InputFocus::POINTER_ROOT, xid, CURRENT_TIME);
    let _ = conn.flush();
    eprintln!("[x11_grab] raise+focus sent, sleeping 80ms...");

    std::thread::sleep(std::time::Duration::from_millis(20));

    eprintln!("[x11_grab] attempting grab_pointer...");
    match conn.grab_pointer(
        true,
        xid,
        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
        xid,
        x11rb::NONE,
        CURRENT_TIME,
    ) {
        Err(e) => eprintln!("[x11_grab] grab_pointer send ERROR: {e}"),
        Ok(cookie) => match cookie.reply() {
            Err(e) => eprintln!("[x11_grab] grab_pointer reply ERROR: {e}"),
            Ok(r) => eprintln!("[x11_grab] grab_pointer status: {:?}", r.status),
        },
    }

    eprintln!("[x11_grab] attempting grab_keyboard...");
    match conn.grab_keyboard(true, xid, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC) {
        Err(e) => eprintln!("[x11_grab] grab_keyboard send ERROR: {e}"),
        Ok(cookie) => match cookie.reply() {
            Err(e) => eprintln!("[x11_grab] grab_keyboard reply ERROR: {e}"),
            Ok(r) => eprintln!("[x11_grab] grab_keyboard status: {:?}", r.status),
        },
    }

    let _ = conn.flush();
    eprintln!("[x11_grab] setup_and_grab DONE");
    Ok(())
}

/// Moves a WM-managed window by sending ConfigureWindow(x, y) to the X server.
/// Must be called AFTER the window is mapped (after show). Bypasses GTK/Tauri so
/// the request goes directly to the WM's ConfigureRequest handler.
pub fn move_window(xid: u32, x: i32, y: i32) {
    eprintln!("[x11_grab] move_window xid={xid} ({x},{y})");
    match connect() {
        Err(e) => eprintln!("[x11_grab] move_window connect failed: {e}"),
        Ok(conn) => {
            let _ = conn.configure_window(xid, &ConfigureWindowAux::new().x(x).y(y));
            let _ = conn.flush();
        }
    }
}

/// Releases pointer and keyboard grabs.
pub fn ungrab_input() -> Result<(), String> {
    eprintln!("[x11_grab] ungrab_input");
    let conn = connect()?;
    let _ = conn.ungrab_pointer(CURRENT_TIME);
    let _ = conn.ungrab_keyboard(CURRENT_TIME);
    let _ = conn.flush();
    Ok(())
}
