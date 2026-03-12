//! X11 input grabbing for the capture overlay.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

fn connect() -> Result<RustConnection, String> {
    RustConnection::connect(None)
        .map(|(conn, _)| conn)
        .map_err(|e| format!("X11 connect: {e}"))
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

    std::thread::sleep(std::time::Duration::from_millis(80));

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

/// Releases pointer and keyboard grabs.
pub fn ungrab_input() -> Result<(), String> {
    eprintln!("[x11_grab] ungrab_input");
    let conn = connect()?;
    let _ = conn.ungrab_pointer(CURRENT_TIME);
    let _ = conn.ungrab_keyboard(CURRENT_TIME);
    let _ = conn.flush();
    Ok(())
}
