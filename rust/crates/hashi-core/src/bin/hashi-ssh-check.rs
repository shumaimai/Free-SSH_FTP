//! 管理下OpenSSHとの接続・転送を確認する開発用CLI。
use anyhow::{Context, Result};
use hashi_core::{
    interaction::{Interaction, Stamp},
    ssh::SshSession,
    transfer,
};
use hashi_storage::config::{HostKey, KnownHosts, Profile};
use serde::Deserialize;
use std::{io::Read, path::PathBuf, sync::Arc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
#[path = "support/compat.rs"]
mod compat;

#[derive(Deserialize)]
struct Request {
    profile: Profile,
    expected_key: HostKey,
    directory: PathBuf,
    #[serde(default)]
    ssh_config: Option<PathBuf>,
    #[serde(default)]
    expected_agent_key: Option<String>,
    #[serde(default)]
    compat_only: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut input = String::new();
    std::io::stdin()
        .take(1024 * 1024)
        .read_to_string(&mut input)?;
    let req: Request = serde_json::from_str(&input)?;
    let resolved = match req.ssh_config.as_deref() {
        Some(path) => hashi_core::ssh_config::resolve_at(&req.profile, path)?,
        None => req.profile.clone(),
    };
    let known: KnownHosts = [(
        format!("{}:{}", resolved.host, resolved.port),
        req.expected_key,
    )]
    .into_iter()
    .collect();
    let (interaction, _requests) = Interaction::new();
    let cancel = CancellationToken::new();
    let session = Arc::new(
        SshSession::connect_with_config(
            &req.profile,
            Stamp::default(),
            known,
            interaction,
            cancel.clone(),
            req.ssh_config.as_deref(),
            None,
        )
        .await?,
    );
    if req.compat_only {
        compat::verify(&session, &req.directory).await?;
        session.background.close();
        session.background.wait().await;
        anyhow::ensure!(session.background.take_errors().is_empty(), "cleanup失敗");
        session.close().await?;
        println!("ssh-sftp-compat-verified");
        return Ok(());
    }
    if let Some(key) = req.expected_agent_key.as_ref() {
        let mut channel = session.shell(80, 24).await?;
        channel.data(&b"ssh-add -L\r"[..]).await?;
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            let mut output = vec![];
            while let Some(message) = channel.wait().await {
                if let russh::ChannelMsg::Data { data } = message {
                    output.extend_from_slice(&data);
                }
                anyhow::ensure!(output.len() <= 1024 * 1024, "agent試験出力が大きすぎます");
                if String::from_utf8_lossy(&output).contains(key) {
                    return anyhow::Ok(());
                }
            }
            anyhow::bail!("転送された合成agent鍵を確認できません")
        })
        .await??;
        channel.close().await?;
    }
    let (status, stdout, _) = session.execute("printf 'HASHI_SSH_TEST\\n'").await?;
    anyhow::ensure!(
        status == 0 && stdout == b"HASHI_SSH_TEST\n",
        "SSHコマンドの結果が一致しません"
    );
    // 締切を超えたexecの後も、同じSSH Transportを使えることを実サーバーで確認。
    let expired = session
        .execute_with_timeout("sleep 2", std::time::Duration::from_millis(50))
        .await;
    anyhow::ensure!(expired.is_err(), "execの締切が効きません");
    let (status, stdout, _) = session.execute("printf 'STILL_ALIVE'").await?;
    anyhow::ensure!(
        status == 0 && stdout == b"STILL_ALIVE",
        "execの締切で接続全体が閉じられました"
    );
    {
        // 操作仲介の独立実行: 終了コード/標準エラー/締切後の接続維持/取消。
        let token = tokio_util::sync::CancellationToken::new();
        let run = session
            .run_command(
                "printf 'OUT'; printf 'ERR' >&2; exit 4",
                std::time::Duration::from_secs(10),
                &token,
                64,
            )
            .await?;
        anyhow::ensure!(
            run.status == "completed"
                && run.exit_code == Some(4)
                && run.output == b"OUT"
                && run.error == b"ERR",
            "独立実行の結果が違います: {run:?}"
        );
        let run = session
            .run_command(
                "printf begin; sleep 5",
                std::time::Duration::from_millis(500),
                &token,
                64,
            )
            .await?;
        anyhow::ensure!(
            run.status == "timeout" && run.exit_code.is_none(),
            "締切が効きません: {run:?}"
        );
        token.cancel();
        let run = session
            .run_command("sleep 5", std::time::Duration::from_secs(10), &token, 64)
            .await?;
        anyhow::ensure!(run.status == "cancelled", "取消が効きません: {run:?}");
        let (status, stdout, _) = session.execute("printf AFTER").await?;
        anyhow::ensure!(
            status == 0 && stdout == b"AFTER",
            "独立実行の後に接続が使えません"
        );
    }
    let nav = session.sftp().await?;
    let xfer = session.sftp().await?;
    {
        // 実ホームのauthorized_keysを変えず、一時の.sshへ登録して追記と重複防止を確認する。
        let ssh_dir = req.directory.join("register-ssh");
        let existing = ssh_dir.join("authorized_keys");
        tokio::fs::create_dir(&ssh_dir).await?;
        tokio::fs::write(&existing, b"ssh-ed25519 AAAAexisting old-without-newline").await?;
        let key = hashi_core::keygen::generate(hashi_core::keygen::KeyKind::Ed25519, "登録試験")?;
        let line = hashi_core::keygen::public_line(&key)?;
        let dir = ssh_dir.to_string_lossy();
        anyhow::ensure!(
            hashi_core::keygen::register_in(&nav, &dir, &line).await?,
            "公開鍵を追加できません"
        );
        anyhow::ensure!(
            !hashi_core::keygen::register_in(&nav, &dir, &line).await?,
            "同じ公開鍵を重複登録しました"
        );
        let written = tokio::fs::read_to_string(&existing).await?;
        anyhow::ensure!(
            written == format!("ssh-ed25519 AAAAexisting old-without-newline\n{line}\n"),
            "authorized_keysの内容が違います: {written:?}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            anyhow::ensure!(
                std::fs::metadata(&existing)?.permissions().mode() & 0o777 == 0o600
                    && std::fs::metadata(&ssh_dir)?.permissions().mode() & 0o777 == 0o700,
                "登録後の権限が違います"
            );
        }
        let fresh = req.directory.join("register-new").join(".ssh");
        tokio::fs::create_dir(fresh.parent().unwrap()).await?;
        anyhow::ensure!(
            hashi_core::keygen::register_in(&nav, &fresh.to_string_lossy(), &line).await?,
            "新しい.sshへ登録できません"
        );
        anyhow::ensure!(
            tokio::fs::read_to_string(fresh.join("authorized_keys")).await? == format!("{line}\n"),
            "新規authorized_keysの内容が違います"
        );
    }
    let source = req.directory.join("source.bin");
    let restored = req.directory.join("restored.bin");
    let remote = req
        .directory
        .join("remote.bin")
        .to_string_lossy()
        .into_owned();
    let bytes: Vec<u8> = (0..1024 * 1024).map(|n| (n % 251) as u8).collect();
    tokio::fs::write(&source, &bytes).await?;
    let (tx, mut rx) = mpsc::channel(16);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let upload_context = hashi_core::transfer_tree::TaskContext {
        job: "upload",
        progress: &tx,
        cancel: &cancel,
        permission: None,
        mode: transfer::Mode::Atomic,
    };
    let upload = transfer::upload(&session, &xfer, &source, &remote, false, &upload_context);
    let directory = req.directory.to_string_lossy();
    let list = transfer::list(&nav, &directory, &cancel);
    // navとxferの独立チャネルで、同時に要求を進める。
    let (uploaded, listed) = tokio::join!(upload, list);
    uploaded?;
    listed?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let server_default = req.directory.join("server-default");
        let expected = xfer
            .open_with_flags_and_attributes(
                server_default.to_string_lossy().as_ref(),
                russh_sftp::protocol::OpenFlags::WRITE
                    | russh_sftp::protocol::OpenFlags::CREATE
                    | russh_sftp::protocol::OpenFlags::EXCLUDE,
                russh_sftp::protocol::FileAttributes::default(),
            )
            .await?;
        expected.close().await?;
        anyhow::ensure!(
            std::fs::metadata(&remote)?.permissions().mode() & 0o777
                == std::fs::metadata(&server_default)?.permissions().mode() & 0o777,
            "新規upload modeがサーバー既定と違います"
        );
        tokio::fs::remove_file(server_default).await?;
    }
    let entries =
        transfer::list_bounded(&session, &req.directory.to_string_lossy(), &cancel).await?;
    anyhow::ensure!(
        entries.iter().any(|entry| entry.name == "remote.bin"),
        "転送中も一覧を読める必要があります"
    );
    transfer::download(
        &xfer,
        &remote,
        &restored,
        transfer::DownloadPolicy::default(),
        "download",
        &tx,
        &cancel,
    )
    .await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let default = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o666))
            .tempfile_in(&req.directory)?;
        anyhow::ensure!(
            std::fs::metadata(&restored)?.permissions().mode() & 0o777
                == default.as_file().metadata()?.permissions().mode() & 0o777,
            "新規download modeがローカル既定と違います"
        );
    }
    anyhow::ensure!(
        tokio::fs::read(&restored).await? == bytes,
        "転送内容が一致しません"
    );
    tokio::fs::write(&restored, &bytes[..65536]).await?;
    transfer::download(
        &xfer,
        &remote,
        &restored,
        transfer::DownloadPolicy {
            overwrite: false,
            resume: true,
            mode: transfer::Mode::Atomic,
        },
        "resume",
        &tx,
        &cancel,
    )
    .await?;
    anyhow::ensure!(
        tokio::fs::read(&restored).await? == bytes,
        "再開した内容が一致しません"
    );
    tokio::fs::write(&remote, &bytes[..65536]).await?;
    transfer::upload_resume(&xfer, &source, &remote, "resume-upload", &tx, &cancel).await?;
    anyhow::ensure!(
        tokio::fs::read(&remote).await? == bytes,
        "upload再開の内容が一致しません"
    );
    verify_atomic_transfers(&session, &xfer, &source, &remote, &restored, &bytes).await?;
    transfer::delete_tree(&nav, &remote, "delete", &cancel).await?;
    let local_tree = req.directory.join("tree");
    let remote_tree = req
        .directory
        .join("remote-tree")
        .to_string_lossy()
        .into_owned();
    let downloaded = req.directory.join("downloaded-tree");
    tokio::fs::create_dir_all(local_tree.join("nested")).await?;
    tokio::fs::write(local_tree.join("日本語.txt"), "日本語\r\n").await?;
    tokio::fs::write(local_tree.join("nested/data.bin"), &bytes[..1024]).await?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(req.directory.join("outside"), local_tree.join("link"))?;
    let context = hashi_core::transfer_tree::TaskContext {
        job: "tree",
        progress: &tx,
        cancel: &cancel,
        permission: None,
        mode: transfer::Mode::Atomic,
    };
    let report = hashi_core::transfer_tree::upload_tree(
        &session,
        &xfer,
        &local_tree,
        &remote_tree,
        hashi_core::transfer_tree::Existing::Refuse,
        &context,
    )
    .await?;
    anyhow::ensure!(report.files == 2, "再帰アップロードの件数が違います");
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        req.directory.join("outside"),
        std::path::Path::new(&remote_tree).join("link"),
    )?;
    let report = hashi_core::transfer_tree::download_tree(
        &session,
        &xfer,
        &remote_tree,
        &downloaded,
        hashi_core::transfer_tree::Existing::Refuse,
        &context,
    )
    .await?;
    anyhow::ensure!(
        report.files == 2
            && tokio::fs::read(downloaded.join("nested/data.bin")).await? == bytes[..1024],
        "再帰ダウンロードの内容が違います"
    );
    anyhow::ensure!(
        tokio::fs::read(downloaded.join("日本語.txt")).await? == "日本語\r\n".as_bytes(),
        "日本語ファイルが変わりました"
    );
    let hits =
        hashi_core::transfer_tree::search(&session, &nav, &remote_tree, "日本語", &cancel).await?;
    anyhow::ensure!(hits.len() == 1, "再帰検索の結果が違います");
    transfer::delete_tree(&nav, &remote_tree, "delete", &cancel).await?;
    drop(tx);
    drain.await.context("進捗処理が終了しませんでした")?;
    let edit_path = req
        .directory
        .join("edit.txt")
        .to_string_lossy()
        .into_owned();
    tokio::fs::write(&edit_path, b"\xef\xbb\xbfhello\r\n").await?;
    let mut document = hashi_core::editor::read_remote(&xfer, &edit_path, &cancel).await?;
    let hashi_core::editor::Body::Text(text) = &mut document.body else {
        anyhow::bail!("text判定が違います")
    };
    text.push_str("日本語\n");
    let bytes = document.payload()?;
    hashi_core::editor::save_remote(
        &session,
        &xfer,
        &edit_path,
        document.original(),
        &bytes,
        &cancel,
    )
    .await?;
    anyhow::ensure!(
        tokio::fs::read(&edit_path).await?
            == b"\xef\xbb\xbfhello\r\n\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\r\n",
        "BOM/CRLF/日本語が変わりました"
    );
    let conflict = hashi_core::editor::save_remote(
        &session,
        &xfer,
        &edit_path,
        document.original(),
        b"must-not-overwrite",
        &cancel,
    )
    .await;
    anyhow::ensure!(
        conflict.is_err() && tokio::fs::read(&edit_path).await? == bytes,
        "外部変更を上書きしました"
    );
    transfer::delete_tree(&nav, &edit_path, "delete", &cancel).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = req.directory.join("permission-data.txt");
        tokio::fs::write(&path, b"permission-fixture").await?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o0))?;
        let journal = Arc::new(hashi_core::permission_journal::Journal::at(
            req.directory.join("permission-journal"),
        )?);
        let backend = Arc::new(hashi_core::permissions::SftpBackend::new(
            session.sftp().await?,
        ));
        let connection = format!(
            "{}@{}:{}",
            session.scope.username, session.scope.host, session.scope.port
        );
        let manager =
            hashi_core::permissions::Manager::new(backend, journal.clone(), connection.clone());
        let remote = path.to_string_lossy();
        let grant = manager.acquire(&remote, 0o444).await?;
        anyhow::ensure!(
            xfer.read(remote.as_ref()).await? == b"permission-fixture",
            "権限付与後の読込が違います"
        );
        grant.release().await?;
        anyhow::ensure!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777 == 0,
            "元の権限が復元されません"
        );
        anyhow::ensure!(
            journal.pending(&connection)?.is_empty(),
            "復元済み記録が残っています"
        );
        let tree = req.directory.join("permission-tree");
        tokio::fs::create_dir_all(tree.join("nested")).await?;
        tokio::fs::write(tree.join("nested/file"), b"nested-permission-fixture").await?;
        for target in [&tree.join("nested/file"), &tree.join("nested"), &tree] {
            std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o0))?;
        }
        let (progress, mut receiver) = tokio::sync::mpsc::channel(128);
        let task = tokio::spawn(async move { while receiver.recv().await.is_some() {} });
        let context = hashi_core::transfer_tree::TaskContext {
            job: "permissions",
            progress: &progress,
            cancel: &cancel,
            permission: Some(&manager),
            mode: transfer::Mode::Atomic,
        };
        let upload_source = req.directory.join("permission-source");
        tokio::fs::write(&upload_source, b"permission-overwrite").await?;
        hashi_core::transfer_tree::upload_tree(
            &session,
            &xfer,
            &upload_source,
            &remote,
            hashi_core::transfer_tree::Existing::Overwrite,
            &context,
        )
        .await?;
        anyhow::ensure!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777 == 0,
            "上書き後の権限が復元されません"
        );
        let new_directory = req.directory.join("permission-new");
        tokio::fs::create_dir(&new_directory).await?;
        std::fs::set_permissions(&new_directory, std::fs::Permissions::from_mode(0o0))?;
        let new_remote = new_directory.join("new-file");
        let upload_result = hashi_core::transfer_tree::upload_tree(
            &session,
            &xfer,
            &upload_source,
            &new_remote.to_string_lossy(),
            hashi_core::transfer_tree::Existing::Refuse,
            &context,
        )
        .await;
        anyhow::ensure!(
            std::fs::metadata(&new_directory)?.permissions().mode() & 0o777 == 0,
            "新規作成後の親権限が復元されません"
        );
        std::fs::set_permissions(&new_directory, std::fs::Permissions::from_mode(0o700))?;
        upload_result?;
        anyhow::ensure!(
            tokio::fs::read(&new_remote).await? == b"permission-overwrite",
            "新規作成した内容が違います"
        );
        tokio::fs::remove_dir_all(new_directory).await?;
        // token取消だけでなくfuture自体のDropでも、temp削除を終えてから親000へ戻す。
        for drop_future in [false, true] {
            let cancelled_directory = req.directory.join(if drop_future {
                "permission-drop"
            } else {
                "permission-cancel"
            });
            tokio::fs::create_dir(&cancelled_directory).await?;
            let preserved_original = cancelled_directory.join("original");
            let cancelled_destination = cancelled_directory.join("new-file");
            tokio::fs::write(&preserved_original, b"keep-original").await?;
            tokio::fs::write(&upload_source, vec![b'x'; 1024 * 1024]).await?;
            std::fs::set_permissions(&cancelled_directory, std::fs::Permissions::from_mode(0o0))?;
            let token = CancellationToken::new();
            let (updates, mut receiver) = mpsc::channel(1);
            let cancellation_context = hashi_core::transfer_tree::TaskContext {
                job: "permission-cancel",
                progress: &updates,
                cancel: &token,
                permission: Some(&manager),
                mode: transfer::Mode::Atomic,
            };
            let destination = cancelled_destination.to_string_lossy();
            {
                let upload = hashi_core::transfer_tree::upload_tree(
                    &session,
                    &xfer,
                    &upload_source,
                    &destination,
                    hashi_core::transfer_tree::Existing::Overwrite,
                    &cancellation_context,
                );
                tokio::pin!(upload);
                tokio::select! {
                    progress=receiver.recv()=>{
                        anyhow::ensure!(progress.is_some(), "取消試験の進捗がありません");
                        if !drop_future {
                            token.cancel();
                            anyhow::ensure!(upload.await.is_err(), "取消が結果へ反映されません");
                        }
                        // drop_futureなら、scopeを抜けてupload自体をDropする。
                    },
                    result=&mut upload=>{result?; anyhow::bail!("取消前に転送が終わりました");},
                }
            }
            if drop_future {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    while !journal.pending(&connection)?.is_empty() {
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                    anyhow::Ok(())
                })
                .await??;
            }
            anyhow::ensure!(
                std::fs::metadata(&cancelled_directory)?
                    .permissions()
                    .mode()
                    & 0o777
                    == 0,
                "取消後の親権限が復元されません"
            );
            std::fs::set_permissions(&cancelled_directory, std::fs::Permissions::from_mode(0o700))?;
            anyhow::ensure!(
                std::fs::read_dir(&cancelled_directory)?.count() == 1,
                "親権限復元後に未削除のtempが残りました"
            );
            anyhow::ensure!(
                tokio::fs::read(&preserved_original).await? == b"keep-original",
                "取消で既存ファイルが変わりました"
            );
            tokio::fs::remove_dir_all(cancelled_directory).await?;
        }
        let readonly_parent = req.directory.join("direct-readonly-parent");
        tokio::fs::create_dir(&readonly_parent).await?;
        let destination = readonly_parent.join("file");
        tokio::fs::write(&destination, b"original").await?;
        use std::os::unix::fs::MetadataExt;
        let inode = std::fs::metadata(&destination)?.ino();
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o0))?;
        std::fs::set_permissions(&readonly_parent, std::fs::Permissions::from_mode(0o500))?;
        let remote = destination.to_string_lossy();
        let refused = hashi_core::transfer_tree::upload_tree(
            &session,
            &xfer,
            &upload_source,
            &remote,
            hashi_core::transfer_tree::Existing::Overwrite,
            &context,
        )
        .await;
        anyhow::ensure!(
            refused.is_err(),
            "原本保持が書込み不可の親を自動拡大しました"
        );
        anyhow::ensure!(
            std::fs::metadata(&readonly_parent)?.permissions().mode() & 0o777 == 0o500,
            "原本保持で親のmodeが変わりました"
        );
        let (writes, mut write_progress) = mpsc::channel(1);
        let direct = hashi_core::transfer_tree::TaskContext {
            job: "direct-permission",
            progress: &writes,
            cancel: &cancel,
            permission: Some(&manager),
            mode: transfer::Mode::InPlace,
        };
        {
            let upload = hashi_core::transfer_tree::upload_tree(
                &session,
                &xfer,
                &upload_source,
                &remote,
                hashi_core::transfer_tree::Existing::Overwrite,
                &direct,
            );
            tokio::pin!(upload);
            tokio::select! {
                update=write_progress.recv()=>{
                    anyhow::ensure!(update.is_some(),"権限中の書込み進捗がありません");
                    anyhow::ensure!(std::fs::metadata(&destination)?.permissions().mode() & 0o777==0o200,"現在の利用者以外へ書込み権限が広がりました");
                    anyhow::ensure!(std::fs::metadata(&readonly_parent)?.permissions().mode() & 0o777==0o500,"転送中に親のmodeを広げました");
                },
                result=&mut upload=>{result?;anyhow::bail!("検証前に転送が完了しました");},
            }
            loop {
                tokio::select! {
                    result=&mut upload=>{result?;break;},
                    _=write_progress.recv()=>{},
                }
            }
        }
        anyhow::ensure!(
            std::fs::metadata(&readonly_parent)?.permissions().mode() & 0o777 == 0o500,
            "直接書込みで親のmodeが変わりました"
        );
        anyhow::ensure!(
            std::fs::metadata(&destination)?.permissions().mode() & 0o777 == 0,
            "対象ファイルのmodeが復元されません"
        );
        anyhow::ensure!(
            std::fs::metadata(&destination)?.ino() == inode,
            "直接書込みでinodeが変わりました"
        );
        anyhow::ensure!(
            journal.pending(&connection)?.is_empty(),
            "直接書込みの復旧記録が残りました"
        );
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o600))?;
        anyhow::ensure!(
            tokio::fs::read(&destination).await? == tokio::fs::read(&upload_source).await?,
            "直接書込みの内容が違います"
        );
        std::fs::set_permissions(&readonly_parent, std::fs::Permissions::from_mode(0o700))?;
        tokio::fs::remove_dir_all(readonly_parent).await?;
        let mixed_local = req.directory.join("mixed-local");
        let mixed_remote = req.directory.join("mixed-remote");
        tokio::fs::create_dir(&mixed_local).await?;
        tokio::fs::create_dir(&mixed_remote).await?;
        tokio::fs::write(mixed_local.join("a"), b"mixed-new").await?;
        tokio::fs::write(mixed_local.join("b"), b"mixed-new").await?;
        // 変更しない二要素のfixtureで、実際のreaddir順から新規→既存を設定する。
        let order = std::fs::read_dir(&mixed_local)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        tokio::fs::write(mixed_remote.join(&order[1]), b"mixed-original").await?;
        std::fs::set_permissions(&mixed_remote, std::fs::Permissions::from_mode(0o500))?;
        let result = hashi_core::transfer_tree::upload_tree(
            &session,
            &xfer,
            &mixed_local,
            &mixed_remote.to_string_lossy(),
            hashi_core::transfer_tree::Existing::Overwrite,
            &context,
        )
        .await;
        anyhow::ensure!(
            result.is_err(),
            "再帰中の借用済み親writeを既存置換に使いました"
        );
        anyhow::ensure!(
            tokio::fs::read(mixed_remote.join(&order[0])).await? == b"mixed-new",
            "新規→既存の試験順が違います"
        );
        anyhow::ensure!(
            tokio::fs::read(mixed_remote.join(&order[1])).await? == b"mixed-original",
            "混在ツリーで既存原本を変えました"
        );
        anyhow::ensure!(
            std::fs::metadata(&mixed_remote)?.permissions().mode() & 0o777 == 0o500,
            "混在ツリーの親modeが戻りません"
        );
        anyhow::ensure!(
            journal.pending(&connection)?.is_empty(),
            "混在ツリーの復旧記録が残りました"
        );
        std::fs::set_permissions(&mixed_remote, std::fs::Permissions::from_mode(0o700))?;
        tokio::fs::remove_dir_all(mixed_remote).await?;
        tokio::fs::remove_dir_all(mixed_local).await?;
        let result = hashi_core::transfer_tree::download_tree(
            &session,
            &xfer,
            &tree.to_string_lossy(),
            &req.directory.join("permission-copy"),
            hashi_core::transfer_tree::Existing::Refuse,
            &context,
        )
        .await;
        anyhow::ensure!(
            std::fs::metadata(&tree)?.permissions().mode() & 0o777 == 0,
            "親権限が復元されません"
        );
        std::fs::set_permissions(&tree, std::fs::Permissions::from_mode(0o700))?;
        anyhow::ensure!(
            std::fs::metadata(tree.join("nested"))?.permissions().mode() & 0o777 == 0,
            "子権限が復元されません"
        );
        std::fs::set_permissions(tree.join("nested"), std::fs::Permissions::from_mode(0o700))?;
        anyhow::ensure!(
            std::fs::metadata(tree.join("nested/file"))?
                .permissions()
                .mode()
                & 0o777
                == 0,
            "ファイル権限が復元されません"
        );
        std::fs::set_permissions(
            tree.join("nested/file"),
            std::fs::Permissions::from_mode(0o600),
        )?;
        result?;
        anyhow::ensure!(
            tokio::fs::read(req.directory.join("permission-copy/nested/file")).await?
                == b"nested-permission-fixture",
            "再帰権限読込が違います"
        );
        anyhow::ensure!(
            journal.pending(&connection)?.is_empty(),
            "再帰復元済み記録が残っています"
        );
        drop(progress);
        task.await?;
        anyhow::ensure!(
            manager.shutdown().await? == 0,
            "終了後に復元記録が残りました"
        );
        tokio::fs::remove_dir_all(tree).await?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        tokio::fs::remove_file(path).await?;
    }
    verify_forwards(session.clone()).await?;
    session.background.close();
    session.background.wait().await;
    anyhow::ensure!(
        session.background.take_errors().is_empty(),
        "後始末に失敗しました"
    );
    anyhow::ensure!(
        transfer::list_bounded(&session, &directory, &cancel)
            .await
            .is_err(),
        "終了後に新しい一覧処理を受け付けました"
    );
    session.close().await?;
    println!("ssh-sftp-verified");
    Ok(())
}

async fn verify_atomic_transfers(
    session: &SshSession,
    sftp: &russh_sftp::client::SftpSession,
    source: &std::path::Path,
    remote: &str,
    local: &std::path::Path,
    bytes: &[u8],
) -> anyhow::Result<()> {
    tokio::fs::write(remote, b"remote-original").await?;
    let cancel = CancellationToken::new();
    let (tx, mut rx) = mpsc::channel(1);
    let context = hashi_core::transfer_tree::TaskContext {
        job: "cancel-upload",
        progress: &tx,
        cancel: &cancel,
        permission: None,
        mode: transfer::Mode::Atomic,
    };
    {
        let upload = transfer::upload(session, sftp, source, remote, true, &context);
        tokio::pin!(upload);
        tokio::select! {
            progress=rx.recv()=>{ anyhow::ensure!(progress.is_some(),"取消試験の進捗がありません");cancel.cancel();anyhow::ensure!(upload.await.is_err(),"upload取消が効きません"); },
            result=&mut upload=>{result?;anyhow::bail!("取消前に試験転送が完了しました");},
        }
    }
    anyhow::ensure!(
        tokio::fs::read(remote).await? == b"remote-original",
        "取消でリモート原本を壊しました"
    );
    let (failed, receiver) = mpsc::channel(1);
    drop(receiver);
    let token = CancellationToken::new();
    let context = hashi_core::transfer_tree::TaskContext {
        job: "failed-upload",
        progress: &failed,
        cancel: &token,
        permission: None,
        mode: transfer::Mode::Atomic,
    };
    anyhow::ensure!(
        transfer::upload(session, sftp, source, remote, true, &context)
            .await
            .is_err(),
        "転送故障が伝わりません"
    );
    anyhow::ensure!(
        tokio::fs::read(remote).await? == b"remote-original",
        "故障でリモート原本を壊しました"
    );
    let (tx, mut rx) = mpsc::channel(128);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let context = hashi_core::transfer_tree::TaskContext {
        job: "atomic-upload",
        progress: &tx,
        cancel: &token,
        permission: None,
        mode: transfer::Mode::Atomic,
    };
    transfer::upload(session, sftp, source, remote, true, &context).await?;
    anyhow::ensure!(
        tokio::fs::read(remote).await? == bytes,
        "確定したupload内容が違います"
    );
    drop(tx);
    drain.await?;
    tokio::fs::write(local, b"local-original").await?;
    let cancel = CancellationToken::new();
    let (tx, mut rx) = mpsc::channel(1);
    let policy = transfer::DownloadPolicy {
        overwrite: true,
        resume: false,
        mode: transfer::Mode::Atomic,
    };
    {
        let download =
            transfer::download(sftp, remote, local, policy, "cancel-download", &tx, &cancel);
        tokio::pin!(download);
        tokio::select! {
            progress=rx.recv()=>{anyhow::ensure!(progress.is_some(),"取消試験の進捗がありません");cancel.cancel();anyhow::ensure!(download.await.is_err(),"download取消が効きません");},
            result=&mut download=>{result?;anyhow::bail!("取消前に試験転送が完了しました");},
        }
    }
    anyhow::ensure!(
        tokio::fs::read(local).await? == b"local-original",
        "取消でローカル原本を壊しました"
    );
    anyhow::ensure!(
        transfer::download(
            sftp,
            remote,
            local,
            policy,
            "failed-download",
            &failed,
            &token
        )
        .await
        .is_err(),
        "転送故障が伝わりません"
    );
    anyhow::ensure!(
        tokio::fs::read(local).await? == b"local-original",
        "故障でローカル原本を壊しました"
    );
    let (tx, mut rx) = mpsc::channel(1);
    let download = transfer::download(
        sftp,
        remote,
        local,
        policy,
        "conflict-download",
        &tx,
        &token,
    );
    tokio::pin!(download);
    let mut changed = false;
    loop {
        tokio::select! {
            progress=rx.recv()=>{
                anyhow::ensure!(progress.is_some(),"競合試験の進捗がありません");
                if !changed {tokio::fs::write(local,b"external-writer").await?;changed=true;}
            },
            result=&mut download=>{anyhow::ensure!(result.is_err(),"競合を拒否しませんでした");break;},
        }
    }
    anyhow::ensure!(
        tokio::fs::read(local).await? == b"external-writer",
        "外部変更を上書きしました"
    );
    let (tx, mut rx) = mpsc::channel(128);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    transfer::download(sftp, remote, local, policy, "atomic-download", &tx, &token).await?;
    anyhow::ensure!(
        tokio::fs::read(local).await? == bytes,
        "確定したdownload内容が違います"
    );
    drop(tx);
    drain.await?;
    Ok(())
}

async fn echo(socket: &mut tokio::net::TcpStream) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    socket.write_all(b"PING").await?;
    let mut bytes = [0; 4];
    socket.read_exact(&mut bytes).await?;
    anyhow::ensure!(&bytes == b"PING", "フォワードの内容が一致しません");
    Ok(())
}
async fn verify_forwards(session: Arc<SshSession>) -> Result<()> {
    use hashi_core::forward::{Destination, LocalForward, RemoteForward};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let target = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await?;
            tasks.spawn(async move {
                let mut bytes = [0; 4];
                socket.read_exact(&mut bytes).await?;
                socket.write_all(&bytes).await?;
                let mut byte = [0];
                anyhow::ensure!(
                    socket.read(&mut byte).await? == 0,
                    "転送後の停止を確認できません"
                );
                anyhow::Ok(())
            });
        }
        while let Some(done) = tasks.join_next().await {
            done??;
        }
        anyhow::Ok(())
    });
    let mut local = LocalForward::start(
        session.clone(),
        "127.0.0.1:0",
        Destination::Fixed {
            host: "127.0.0.1".into(),
            port: target.port(),
        },
    )
    .await?;
    let mut socket = TcpStream::connect(local.address).await?;
    echo(&mut socket).await?;
    local.stop().await;
    let mut byte = [0];
    anyhow::ensure!(
        tokio::time::timeout(std::time::Duration::from_secs(2), socket.read(&mut byte)).await??
            == 0,
        "停止後の-Lチャネルが残っています"
    );
    let mut dynamic =
        LocalForward::start(session.clone(), "127.0.0.1:0", Destination::Socks5).await?;
    let mut socket = TcpStream::connect(dynamic.address).await?;
    socket.write_all(&[5, 1, 0]).await?;
    let mut response = [0; 2];
    socket.read_exact(&mut response).await?;
    anyhow::ensure!(response == [5, 0], "SOCKS5方式が違います");
    let mut request = vec![5, 1, 0, 3, 9];
    request.extend_from_slice(b"localhost");
    request.extend_from_slice(&target.port().to_be_bytes());
    socket.write_all(&request).await?;
    let mut response = [0; 10];
    socket.read_exact(&mut response).await?;
    anyhow::ensure!(response[1] == 0, "SOCKS5接続を拒否されました");
    echo(&mut socket).await?;
    dynamic.stop().await;
    let remote = RemoteForward::start(
        session.clone(),
        "127.0.0.1".into(),
        0,
        "127.0.0.1".into(),
        target.port(),
    )
    .await?;
    let mut socket = TcpStream::connect(("127.0.0.1", remote.port as u16)).await?;
    echo(&mut socket).await?;
    remote.stop().await?;
    server.await??;
    let (status, stdout, _) = session.execute("printf 'FORWARDS_STOPPED'").await?;
    anyhow::ensure!(
        status == 0 && stdout == b"FORWARDS_STOPPED",
        "フォワード停止がSSH接続を巻き込みました"
    );
    Ok(())
}
