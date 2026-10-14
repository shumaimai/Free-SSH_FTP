use std::{io, path::PathBuf};

fn main() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let directory = match args.next() {
        Some(value) => PathBuf::from(value),
        None => hashi_storage::paths::runtime_root()?,
    };
    let hold = match args.next() {
        Some(value) if value == "--hold" => true,
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "不明な引数です",
            ))
        }
        None => false,
    };
    if args.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "引数が多すぎます",
        ));
    }
    let lease = hashi_storage::migration::acquire(&directory)?;
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
