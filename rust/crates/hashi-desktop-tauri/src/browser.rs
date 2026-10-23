//! 外部ブラウザでURLを開く。画面からURLを受け取らず、Rust側が決めたhttp(s)のURLだけを渡す。
//! Windowsではエクスプローラと同じ`ShellExecuteW`で開く。`webbrowser`クレートは既定ブラウザの
//! コマンド行を自分で組み立てるため、DelegateExecute型(Store版など)のブラウザで開けないことがある。

/// http(s)のURLだけを既定のブラウザで開く。
pub fn open(url: &str) -> Result<(), String> {
    let url = checked(url)?;
    open_checked(&url).map_err(|error| format!("ブラウザを開けません: {error}"))
}

/// 開いてよいURLか確かめる。ShellExecuteは任意の文字列を実行ファイルとして起動し得るため、
/// http(s)以外は渡さない。
fn checked(url: &str) -> Result<String, String> {
    let parsed = url::Url::parse(url).map_err(|_| "ブラウザで開けないURLです".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("http(s)以外のURLは開きません".into());
    }
    Ok(parsed.into())
}

#[cfg(windows)]
fn open_checked(url: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let file: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    // DelegateExecute型のブラウザはCOM経由で起動されるため、STAで初期化した専用スレッドで呼ぶ
    // (Tauriのworkerスレッドや非同期処理の中ではCOMの状態が分からない)。
    std::thread::spawn(move || {
        // SAFETY: 引数はどれもNUL終端したUTF-16で、呼び出しの間このスレッドが所有している。
        // CoInitializeExが成功したときだけ、同じスレッドでCoUninitializeを呼ぶ。
        unsafe {
            let com = CoInitializeEx(
                std::ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            );
            let result = ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            ) as isize;
            if com >= 0 {
                CoUninitialize();
            }
            // 32以下は失敗(ShellExecuteWの仕様)。
            if result > 32 {
                Ok(())
            } else {
                Err(format!("ShellExecute {result}"))
            }
        }
    })
    .join()
    .map_err(|_| "ブラウザの起動処理が異常終了しました".to_string())?
}

#[cfg(not(windows))]
fn open_checked(url: &str) -> Result<(), String> {
    webbrowser::open(url).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_urls_with_a_host_are_opened() {
        assert_eq!(
            checked("https://auth.openai.com/oauth/authorize?a=1&b=2").unwrap(),
            "https://auth.openai.com/oauth/authorize?a=1&b=2"
        );
        assert!(checked("http://127.0.0.1:1455/x").is_ok());
        for bad in [
            "file:///C:/Windows/System32/calc.exe",
            r"C:\Windows\System32\calc.exe",
            "calc.exe",
            "javascript:alert(1)",
            "ms-settings:",
            "https://",
            "",
        ] {
            assert!(checked(bad).is_err(), "{bad}");
        }
    }
}
