use std::{io, path::PathBuf};

fn main() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let directory = match args.next() {
        Some(value) => PathBuf::from(value),
        None => hashi_storage::paths::runtime_root()?,
    };
    let mut hold = false;
    let mut legacy_root = None;
    while let Some(value) = args.next() {
        if value == "--hold" {
            hold = true;
        } else if value == "--legacy-root" {
            legacy_root = Some(PathBuf::from(args.next().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "旧保存先を指定してください")
            })?));
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "不明な引数です",
            ));
        }
    }
    let lease = hashi_storage::migration::acquire(&directory)?;
    if let Some(root) = legacy_root {
        let status = hashi_storage::legacy_permissions::inspect(&root, &lease.stale_markers)?;
        status.require_clear()?;
        eprintln!(
            "旧復旧記録の確認先: {}件、未復元0件（読取のみ）",
            status.paths.len()
        );
    }
    println!("guard-acquired");
    if hold {
        use std::io::Write;
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
    }
    eprintln!(
        "残存マーカー: {}件（未対応の旧版停止は別途確認が必要）",
        lease.stale_markers.len()
    );
    Ok(())
}
