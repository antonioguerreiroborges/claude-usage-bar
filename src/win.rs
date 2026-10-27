//! Tudo o que é específico do Windows: estilo da janela, posição, menu de
//! contexto nativo, instância única, iniciar com o Windows e DPAPI.

use std::ffi::c_void;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, LocalFree, BOOL, ERROR_ALREADY_EXISTS, HLOCAL, HWND, LPARAM, POINT, RECT,
    WPARAM,
};
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONULL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetForegroundWindow,
    GetWindowLongPtrW, GetWindowRect, PostMessageW, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, SystemParametersInfoW, TrackPopupMenu, GWL_EXSTYLE, HWND_TOPMOST, MF_CHECKED,
    MF_GRAYED, MF_SEPARATOR, MF_STRING, SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, TPM_BOTTOMALIGN, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_NULL, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "UsageBarRs";
const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

fn h(hwnd: isize) -> HWND {
    HWND(hwnd as *mut c_void)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `false` quando outra instância já tem o mutex. O handle fica aberto até o
/// processo acabar.
pub fn single_instance() -> bool {
    unsafe {
        match CreateMutexW(None, BOOL(0), w!("UsageBarRs_SingleInstance")) {
            Ok(_) => GetLastError() != ERROR_ALREADY_EXISTS,
            Err(_) => true,
        }
    }
}

/// Fora do Alt-Tab e sem roubar o foco de quem está trabalhando.
pub fn make_tool_window(hwnd: isize) {
    unsafe {
        let ex = GetWindowLongPtrW(h(hwnd), GWL_EXSTYLE);
        let add = (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) as isize;
        let remove = WS_EX_APPWINDOW.0 as isize;
        SetWindowLongPtrW(h(hwnd), GWL_EXSTYLE, (ex | add) & !remove);
    }
}

pub fn keep_topmost(hwnd: isize) {
    unsafe {
        let _ = SetWindowPos(
            h(hwnd),
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// (x, y, largura, altura) em pixels físicos.
pub fn window_rect(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(h(hwnd), &mut r).ok()? };
    Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

fn on_screen(x: i32, y: i32) -> bool {
    unsafe { !MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL).is_invalid() }
}

fn work_area() -> RECT {
    let mut r = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut r as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    r
}

/// Coloca a janela onde estava, ou no canto inferior direito da área de
/// trabalho do monitor principal quando o lugar salvo não está mais em
/// nenhum monitor (monitor desconectado, resolução mudou). Só move: o
/// tamanho é lógico e o winit o ajusta à escala do monitor de destino.
pub fn place(hwnd: isize, x: Option<i32>, y: Option<i32>) {
    let Some((_, _, w, cur_h)) = window_rect(hwnd) else {
        return;
    };
    let (x, y) = match (x, y) {
        (Some(x), Some(y)) if on_screen(x, y) && on_screen(x + w - 1, y + cur_h - 1) => (x, y),
        _ => {
            let wa = work_area();
            (wa.right - w - 16, wa.bottom - cur_h - 8)
        }
    };
    unsafe {
        let _ = SetWindowPos(h(hwnd), HWND_TOPMOST, x, y, 0, 0, SWP_NOACTIVATE | SWP_NOSIZE);
    }
}

/// Estado real do botão esquerdo. O egui não vê o botão ser solto dentro do
/// loop modal do Windows de mover ou redimensionar.
pub fn left_button_down() -> bool {
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 != 0 }
}

pub enum MenuItem<'a> {
    Info(&'a str),
    Action { id: u32, text: &'a str, checked: bool },
    Separator,
}

/// Mostra o menu no cursor e devolve o id escolhido (0 = nada).
pub fn popup_menu(hwnd: isize, items: &[MenuItem]) -> u32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return 0;
        };
        let mut texts: Vec<Vec<u16>> = Vec::new();
        for item in items {
            match item {
                MenuItem::Separator => {
                    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                }
                MenuItem::Info(text) => {
                    texts.push(wide(text));
                    let p = PCWSTR(texts.last().unwrap().as_ptr());
                    let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, p);
                }
                MenuItem::Action { id, text, checked } => {
                    texts.push(wide(text));
                    let p = PCWSTR(texts.last().unwrap().as_ptr());
                    let flags = if *checked { MF_STRING | MF_CHECKED } else { MF_STRING };
                    let _ = AppendMenuW(menu, flags, *id as usize, p);
                }
            }
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // O menu só fecha com um clique fora se a janela dona estiver em
        // primeiro plano; depois, o foco volta para quem o tinha.
        let previous = GetForegroundWindow();
        let _ = SetForegroundWindow(h(hwnd));
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            0,
            h(hwnd),
            None,
        );
        let _ = PostMessageW(h(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        if !previous.is_invalid() {
            let _ = SetForegroundWindow(previous);
        }
        cmd.0 as u32
    }
}

fn exe_command() -> String {
    format!(
        "\"{}\"",
        std::env::current_exe().unwrap_or_default().display()
    )
}

pub fn autostart_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|k| k.get_value::<String, _>(RUN_VALUE))
        .map(|v| v.eq_ignore_ascii_case(&exe_command()))
        .unwrap_or(false)
}

pub fn set_autostart(on: bool) -> std::io::Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if on {
        key.set_value(RUN_VALUE, &exe_command())
    } else {
        match key.delete_value(RUN_VALUE) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }
}

unsafe fn take_blob(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let v = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec();
    let _ = LocalFree(HLOCAL(blob.pbData as *mut c_void));
    v
}

/// Criptografa para o usuário atual do Windows (DPAPI).
pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
        .map_err(|e| e.to_string())?;
        Ok(take_blob(out))
    }
}

pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
        .map_err(|e| e.to_string())?;
        Ok(take_blob(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpapi_round_trip() {
        let secret = br#"{"refreshToken":"rt-123"}"#;
        let sealed = protect(secret).unwrap();
        assert_ne!(sealed.as_slice(), secret.as_slice());
        assert_eq!(unprotect(&sealed).unwrap(), secret);
    }

    #[test]
    fn dpapi_rejects_garbage() {
        assert!(unprotect(b"not a dpapi blob").is_err());
    }
}
