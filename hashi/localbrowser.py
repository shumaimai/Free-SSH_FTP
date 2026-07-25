"""ローカル側ファイルブラウザ(Issue #82)。

WinSCP 流のデュアルペインの**左側**。右側の `filebrowser.SftpBrowser` と
列構成・ツールバー・余白を揃えてある(#87 / docs/ui-style-guide.md)。

`filebrowser.py` は最大のファイルなので、ローカル側はこの新モジュールに
分離している。SftpBrowser 側への変更は「転送の受け口(`upload_paths` /
`download_to`)」と「ドラッグ元になること」だけに留めること。

スレッドについて:
- SFTP と違い、ローカル FS の操作は OS 呼び出しなので **GUI スレッドで直接**
  行う。専用ワーカー/チャネルは持たない。
- ただし応答が返らないネットワークドライブ(切れた SMB 共有など)を開くと
  一覧取得で固まりうる。**実機での確認はまだ**(PR に明記)。

転送の実体は必ずリモート側(SftpWorker の xfer チャネル)に載せる。ここは
「どのローカルパスを送る / どこへ受け取る」をシグナルで伝えるだけにする。
"""
from __future__ import annotations

import logging
import os
import shutil
import stat as statmod
import subprocess
import sys
from html import escape

from PySide6.QtCore import QFileSystemWatcher, QMimeData, Qt, QTimer, QUrl, Signal
from PySide6.QtGui import QDesktopServices, QKeySequence, QShortcut
from PySide6.QtWidgets import (
    QAbstractItemView,
    QFileDialog,
    QHBoxLayout,
    QInputDialog,
    QLineEdit,
    QMenu,
    QMessageBox,
    QStyle,
    QToolButton,
    QTreeWidget,
    QVBoxLayout,
    QWidget,
)

from . import style
from .dialogs import DoubleCheckDialog
from .filebrowser import REMOTE_DRAG_MIME, _SortItem, fmt_mtime, human_size

logger = logging.getLogger(__name__)


def is_hidden(name: str, path: str = "") -> bool:
    """隠しファイル判定。ドットファイル + Windows の隠し属性。"""
    if name.startswith("."):
        return True
    if os.name == "nt" and path:
        flag = getattr(statmod, "FILE_ATTRIBUTE_HIDDEN", 0)
        try:
            return bool(os.stat(path).st_file_attributes & flag)
        except (OSError, AttributeError):
            return False
    return False


def scan_dir(path: str) -> list[dict]:
    """ローカルディレクトリを一覧する。壊れたリンクや権限不足でも落とさない。

    返す辞書のキーはリモート側 (`SftpWorker._job_list`) と同じ形にしてある。
    列の描画コードを揃えるため。
    """
    entries: list[dict] = []
    with os.scandir(path) as it:
        for de in it:
            try:
                st = de.stat(follow_symlinks=False)
                mode = st.st_mode
                size = st.st_size
                mtime = st.st_mtime
            except OSError:
                # 壊れたシンボリックリンク等。行は出すが情報は空にする
                logger.debug("stat に失敗 (一覧には残す): %s", de.path, exc_info=True)
                mode, size, mtime = 0, 0, 0
            is_link = statmod.S_ISLNK(mode)
            try:
                is_dir = de.is_dir()      # リンクは辿った先で判定(表示上の見た目)
            except OSError:
                is_dir = statmod.S_ISDIR(mode)
            entries.append({
                "name": de.name,
                "path": de.path,
                "is_dir": is_dir,
                "is_link": is_link,
                "size": size,
                "mtime": mtime,
                "mode_str": statmod.filemode(mode) if mode else "",
            })
    return entries


def delete_local_path(path: str) -> None:
    """ローカルの 1 項目を削除する。

    🔴 **シンボリックリンクは中身を辿らない。** `os.path.isdir()` はリンク先が
    ディレクトリなら True になるため、先に islink を見ないと `shutil.rmtree` が
    **リンク先の実体を消してしまう**。
    """
    if os.path.islink(path) or os.path.isfile(path):
        os.unlink(path)
    elif os.path.isdir(path):
        shutil.rmtree(path)
    elif os.path.lexists(path):
        os.unlink(path)


def open_in_file_manager(path: str) -> bool:
    """OS のファイルマネージャで開く(Windows: エクスプローラ)。"""
    try:
        if os.name == "nt":
            os.startfile(path)  # noqa: S606 - ユーザーが選んだ自分のフォルダ
            return True
        if sys.platform == "darwin":
            subprocess.Popen(["open", path])
            return True
    except OSError:
        logger.debug("ファイルマネージャの起動に失敗: %s", path, exc_info=True)
        return False
    return QDesktopServices.openUrl(QUrl.fromLocalFile(path))


class _LocalTree(QTreeWidget):
    """ローカル一覧。外(リモートペイン / OS)へのドラッグ元になる。

    リモートペインからのドロップは「ダウンロード要求」として扱う。ドラッグに
    載っているのは印だけなので、**リモート由来のデータは一切解釈しない**
    (何を落とすかはリモートブラウザの選択状態が決める)。
    """

    remote_dropped = Signal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setAcceptDrops(True)
        self.setDragEnabled(True)
        self.setDragDropMode(QAbstractItemView.DragDrop)
        self.setDefaultDropAction(Qt.CopyAction)
        self.setDropIndicatorShown(False)

    def mimeData(self, items):
        md = QMimeData()
        urls = []
        for it in items:
            entry = it.data(0, Qt.UserRole + 2) or {}
            path = entry.get("path")
            if path:
                urls.append(QUrl.fromLocalFile(path))
        if urls:
            md.setUrls(urls)
        return md

    def dragEnterEvent(self, ev):
        self._accept_remote_drag(ev)

    def dragMoveEvent(self, ev):
        self._accept_remote_drag(ev)

    @staticmethod
    def _accept_remote_drag(ev):
        if ev.mimeData().hasFormat(REMOTE_DRAG_MIME):
            ev.acceptProposedAction()
        else:
            # 自分自身から始まったドラッグ(URL)は受けない
            ev.ignore()

    def dropEvent(self, ev):
        if ev.mimeData().hasFormat(REMOTE_DRAG_MIME):
            self.remote_dropped.emit()
            ev.acceptProposedAction()
            return
        ev.ignore()


class LocalBrowser(QWidget):
    """ローカル側のファイル一覧ペイン。

    転送は自分では行わず、シグナルでリモート側へ依頼する:
    - `upload_requested(paths)`   : 選択したローカル項目をリモートの現在地へ
    - `download_requested(dest)`  : リモートの選択項目を dest(ここの現在地)へ
    """

    upload_requested = Signal(list)     # list[str] ローカルパス
    download_requested = Signal(str)    # 保存先ローカルディレクトリ
    status_message = Signal(str)
    path_changed = Signal(str)

    def __init__(self, settings=None, start_dir: str = "", parent=None):
        super().__init__(parent)
        self.settings = settings
        self.cwd = ""
        self.home = os.path.expanduser("~")
        self._entries: list[dict] = []
        self._show_hidden = False

        self._build_ui()

        # 現在のフォルダを監視して自動更新(アップロード/ダウンロードの結果が
        # すぐ見えるように)。頻発するので短いデバウンスを噛ませる。
        self._watcher = QFileSystemWatcher(self)
        self._watcher.directoryChanged.connect(self._schedule_refresh)
        self._refresh_timer = QTimer(self)
        self._refresh_timer.setSingleShot(True)
        self._refresh_timer.timeout.connect(self.refresh)

        self.cd(start_dir or self.home)

    # ---- UI 構築 ---------------------------------------------------------
    def _build_ui(self):
        root = QVBoxLayout(self)
        root.setContentsMargins(4, 4, 4, 4)
        root.setSpacing(4)

        qstyle = self.style()

        def icon_button(tip, slot, standard_icon):
            b = QToolButton()
            b.setToolTip(tip)
            b.setIcon(qstyle.standardIcon(standard_icon))
            b.clicked.connect(slot)
            return b

        bar = QHBoxLayout()
        bar.setSpacing(4)
        bar.addWidget(icon_button("1つ上のフォルダへ (Backspace)", self.go_up,
                                  QStyle.SP_ArrowUp))
        bar.addWidget(icon_button("最新の状態に更新 (F5)", self.refresh,
                                  QStyle.SP_BrowserReload))

        self.ed_path = QLineEdit()
        self.ed_path.setPlaceholderText("ローカルパス")
        self.ed_path.returnPressed.connect(self._path_entered)
        bar.addWidget(self.ed_path, 1)

        self.btn_hidden = QToolButton()
        self.btn_hidden.setToolTip("隠しファイルの表示/非表示")
        self.btn_hidden.setIcon(style.icon("eye"))
        self.btn_hidden.setCheckable(True)
        self.btn_hidden.toggled.connect(self._toggle_hidden)
        bar.addWidget(self.btn_hidden)

        # 転送ボタン(#82)。矢印は**ペインの位置関係**を表す(左=ローカル /
        # 右=リモート)ので、↑↓ のアイコンは付けず文字の → ← だけにする。
        self.btn_upload = QToolButton()
        self.btn_upload.setText("→ アップロード")
        self.btn_upload.setToolTip(
            "選択したローカルの項目をリモートの現在のフォルダへ送ります\n"
            "(リモート側ペインへのドラッグ&ドロップでも同じことができます)")
        self.btn_upload.clicked.connect(self._request_upload)
        bar.addWidget(self.btn_upload)

        self.btn_download = QToolButton()
        self.btn_download.setText("← ダウンロード")
        self.btn_download.setToolTip(
            "リモート側で選択中の項目をこのフォルダへ受け取ります\n"
            "(リモート側からこのペインへドラッグ&ドロップでも同じ)")
        self.btn_download.clicked.connect(self._request_download)
        bar.addWidget(self.btn_download)

        self.btn_more = QToolButton()
        self.btn_more.setText("その他")
        self.btn_more.setToolButtonStyle(Qt.ToolButtonTextBesideIcon)
        self.btn_more.setPopupMode(QToolButton.InstantPopup)
        self.btn_more.setMinimumWidth(50)
        menu = QMenu(self)
        menu.addAction("ホームへ", self.go_home)
        menu.addAction("フォルダを選択…", self._pick_dir)
        menu.addAction("ファイルマネージャで開く", self._open_in_file_manager)
        menu.addSeparator()
        menu.addAction("新規フォルダ…", self.make_dir)
        menu.addAction("新規ファイル…", self.make_file)
        self.btn_more.setMenu(menu)
        bar.addWidget(self.btn_more)
        root.addLayout(bar)

        self.tree = _LocalTree()
        self.tree.setHeaderLabels(["名前", "サイズ", "更新日時", "属性"])
        self.tree.setRootIsDecorated(False)
        self.tree.setSortingEnabled(True)
        self.tree.sortByColumn(0, Qt.AscendingOrder)
        self.tree.setSelectionMode(QAbstractItemView.ExtendedSelection)
        self.tree.setContextMenuPolicy(Qt.CustomContextMenu)
        self.tree.customContextMenuRequested.connect(self._context_menu)
        self.tree.itemDoubleClicked.connect(self._double_clicked)
        self.tree.remote_dropped.connect(self._request_download)
        self.tree.setColumnWidth(0, 260)
        self.tree.setColumnWidth(1, 90)
        self.tree.setColumnWidth(2, 130)
        self.tree.setMinimumWidth(120)
        root.addWidget(self.tree, 1)

        bottom = QHBoxLayout()
        bottom.setSpacing(4)
        # ローカルのファイル名・OS のエラー文が入るので PlainText 固定(#113)
        self.lb_status = style.plain_label("", muted=True)
        bottom.addWidget(self.lb_status, 1)
        root.addLayout(bottom)

        self._status_timer = QTimer(self)
        self._status_timer.setSingleShot(True)
        self._status_timer.timeout.connect(lambda: self.lb_status.setText(""))

        # ショートカットはツリーにフォーカスがあるときだけ効かせる
        # (ターミナル入力の Backspace 等を横取りしないため。リモート側と同じ)
        for key, slot in ((Qt.Key_F5, self.refresh),
                          (Qt.Key_Delete, self.delete_selected),
                          (Qt.Key_F2, self.rename_selected),
                          (Qt.Key_Backspace, self.go_up)):
            sc = QShortcut(QKeySequence(key), self.tree, slot)
            sc.setContext(Qt.WidgetWithChildrenShortcut)

    # ---- ナビゲーション -----------------------------------------------------
    def cd(self, path: str) -> None:
        path = os.path.abspath(os.path.expanduser(path or self.home))
        if not os.path.isdir(path):
            self._status(f"フォルダが見つかりません: {path}")
            return
        try:
            entries = scan_dir(path)
        except OSError as e:
            self._status(f"開けません: {e}")
            return
        if self.cwd and self.cwd in self._watcher.directories():
            self._watcher.removePath(self.cwd)
        self.cwd = path
        self._entries = entries
        self.ed_path.setText(path)
        self._watcher.addPath(path)
        self._render()
        self.path_changed.emit(path)

    def refresh(self) -> None:
        if not self.cwd:
            return
        try:
            self._entries = scan_dir(self.cwd)
        except OSError as e:
            self._status(f"更新できません: {e}")
            return
        self._render()

    def _schedule_refresh(self, _path: str = "") -> None:
        self._refresh_timer.start(400)

    def go_up(self) -> None:
        if not self.cwd:
            return
        parent = os.path.dirname(self.cwd.rstrip(os.sep + "/")) or self.cwd
        if parent != self.cwd:
            self.cd(parent)

    def go_home(self) -> None:
        self.cd(self.home)

    def _path_entered(self) -> None:
        path = self.ed_path.text().strip()
        if path:
            self.cd(path)

    def _pick_dir(self) -> None:
        d = QFileDialog.getExistingDirectory(self, "フォルダを選択",
                                             self.cwd or self.home)
        if d:
            self.cd(d)

    def _open_in_file_manager(self) -> None:
        if self.cwd and not open_in_file_manager(self.cwd):
            self._status("ファイルマネージャを開けませんでした")

    # ---- 表示 --------------------------------------------------------------
    def _toggle_hidden(self, on: bool) -> None:
        self._show_hidden = on
        self.btn_hidden.setIcon(
            style.icon("eye", style.ACCENT if on else style.FG))
        self._render()

    def _render(self) -> None:
        self.tree.setSortingEnabled(False)
        self.tree.clear()
        qstyle = self.style()
        icon_dir = qstyle.standardIcon(QStyle.SP_DirIcon)
        icon_file = qstyle.standardIcon(QStyle.SP_FileIcon)
        icon_link = qstyle.standardIcon(QStyle.SP_FileLinkIcon)
        for e in self._entries:
            if not self._show_hidden and is_hidden(e["name"], e["path"]):
                continue
            item = _SortItem([
                e["name"],
                "" if e["is_dir"] else human_size(e["size"]),
                fmt_mtime(e["mtime"]),
                e["mode_str"],
            ])
            item.setIcon(0, icon_dir if e["is_dir"]
                         else (icon_link if e["is_link"] else icon_file))
            item.setData(0, Qt.UserRole, e["name"].lower())
            item.setData(0, Qt.UserRole + 1, e["is_dir"])
            item.setData(0, Qt.UserRole + 2, e)
            item.setData(1, Qt.UserRole, e["size"] or 0)
            item.setData(2, Qt.UserRole, e["mtime"] or 0)
            item.setTextAlignment(1, Qt.AlignRight | Qt.AlignVCenter)
            self.tree.addTopLevelItem(item)
        self.tree.setSortingEnabled(True)

    def _selected_entries(self) -> list[dict]:
        return [it.data(0, Qt.UserRole + 2) for it in self.tree.selectedItems()]

    def selected_paths(self) -> list[str]:
        return [e["path"] for e in self._selected_entries() if e]

    def _double_clicked(self, item, _col) -> None:
        e = item.data(0, Qt.UserRole + 2)
        if not e:
            return
        if e["is_dir"]:
            self.cd(e["path"])
            return
        if not QDesktopServices.openUrl(QUrl.fromLocalFile(e["path"])):
            self._status(f"開けませんでした: {e['name']}")

    # ---- 転送の依頼 ---------------------------------------------------------
    def _request_upload(self) -> None:
        paths = self.selected_paths()
        if not paths:
            self._status("アップロードする項目を選択してください")
            return
        self.upload_requested.emit(paths)

    def _request_download(self) -> None:
        if not self.cwd:
            return
        self.download_requested.emit(self.cwd)

    # ---- ローカルの操作 ------------------------------------------------------
    def make_dir(self) -> None:
        name, ok = QInputDialog.getText(self, "新規フォルダ", "フォルダ名:")
        name = (name or "").strip()
        if not (ok and name):
            return
        if not self._valid_name(name, "フォルダ名"):
            return
        try:
            os.mkdir(os.path.join(self.cwd, name))
        except OSError as e:
            QMessageBox.warning(self, "新規フォルダ", f"作成できません:\n{e}")
            return
        self._status(f"フォルダを作成しました: {name}")
        self.refresh()

    def make_file(self) -> None:
        name, ok = QInputDialog.getText(
            self, "新規ファイル", "ファイル名 (拡張子込み。例: memo.txt):")
        name = (name or "").strip()
        if not (ok and name):
            return
        if not self._valid_name(name, "ファイル名"):
            return
        try:
            # "x" は既存ファイルがあれば失敗する(既存を壊さない)
            with open(os.path.join(self.cwd, name), "x"):
                pass
        except OSError as e:
            QMessageBox.warning(self, "新規ファイル", f"作成できません:\n{e}")
            return
        self._status(f"ファイルを作成しました: {name}")
        self.refresh()

    def rename_selected(self) -> None:
        sel = self._selected_entries()
        if len(sel) != 1:
            return
        e = sel[0]
        new, ok = QInputDialog.getText(self, "名前の変更", "新しい名前:",
                                       text=e["name"])
        new = (new or "").strip()
        if not (ok and new) or new == e["name"]:
            return
        if not self._valid_name(new, "名前"):
            return
        target = os.path.join(self.cwd, new)
        if os.path.lexists(target):
            QMessageBox.warning(self, "名前の変更",
                                f"変更できません: {new} は既に存在します。")
            return
        try:
            os.rename(e["path"], target)
        except OSError as err:
            QMessageBox.warning(self, "名前の変更", f"変更できません:\n{err}")
            return
        self._status("名前を変更しました")
        self.refresh()

    def _valid_name(self, name: str, what: str) -> bool:
        if "/" in name or "\\" in name or name in (".", ".."):
            QMessageBox.warning(self, what, f"{what}に使えない文字が含まれています。")
            return False
        return True

    def delete_selected(self) -> None:
        """ローカル項目の削除。リモート側と同じく 2 段階確認を必須にする。

        ローカルの削除はゴミ箱を経由しない(依存を増やさないため)。
        取り返しがつかないので確認は省略しない。
        """
        sel = [e for e in self._selected_entries() if e]
        if not sel:
            return
        names = [e["name"] for e in sel]
        shown = "<br>".join(f"・{escape(n)}" for n in names[:8])
        if len(names) > 8:
            shown += f"<br>… ほか {len(names) - 8} 件"
        box = QMessageBox(self)
        box.setWindowTitle("ローカル削除の確認 (1/2)")
        box.setTextFormat(Qt.RichText)
        box.setIcon(QMessageBox.Warning)
        box.setText(
            f"この PC から <b>{len(names)} 個</b> の項目を削除します。"
            f"<br><br>{shown}<br><br>"
            "フォルダは中身ごと削除され、ゴミ箱には入りません。"
        )
        next_btn = box.addButton("次へ", QMessageBox.AcceptRole)
        box.addButton("キャンセル", QMessageBox.RejectRole)
        box.exec()
        if box.clickedButton() is not next_btn:
            return
        if not DoubleCheckDialog.confirm(
            self, "ローカル削除の確認 (2/2)",
            f"<b style='color:{style.ERROR};'>この操作は取り消せません。</b>",
            "delete", "削除する",
        ):
            return
        failed: list[str] = []
        for e in sel:
            try:
                delete_local_path(e["path"])
            except OSError as err:
                logger.warning("ローカル削除に失敗: %s", e["path"], exc_info=True)
                failed.append(f"{e['name']}: {err}")
        self.refresh()
        if failed:
            QMessageBox.warning(
                self, "ローカル削除",
                "削除できなかった項目があります:\n" + "\n".join(failed[:8]))
            return
        self._status(f"削除完了 ({len(sel)} 項目)")

    # ---- コンテキストメニュー --------------------------------------------------
    def _context_menu(self, pos) -> None:
        sel = self._selected_entries()
        menu = QMenu(self)
        a_up = menu.addAction("→ リモートへアップロード")
        a_dl = menu.addAction("← リモートからここへダウンロード")
        menu.addSeparator()
        a_open = menu.addAction("関連付けアプリで開く")
        a_ren = menu.addAction("名前の変更 (F2)")
        a_del = menu.addAction("削除 (Del)")
        menu.addSeparator()
        a_new = menu.addAction("新規フォルダ…")
        a_newfile = menu.addAction("新規ファイル…")
        a_copy = menu.addAction("ローカルパスをコピー")
        a_ref = menu.addAction("更新 (F5)")
        one_file = len(sel) == 1 and not sel[0]["is_dir"]
        a_up.setEnabled(bool(sel))
        a_open.setEnabled(one_file)
        a_ren.setEnabled(len(sel) == 1)
        a_del.setEnabled(bool(sel))
        a_copy.setEnabled(bool(sel) or bool(self.cwd))
        chosen = menu.exec(self.tree.viewport().mapToGlobal(pos))
        if chosen is a_up:
            self._request_upload()
        elif chosen is a_dl:
            self._request_download()
        elif chosen is a_open and one_file:
            QDesktopServices.openUrl(QUrl.fromLocalFile(sel[0]["path"]))
        elif chosen is a_ren:
            self.rename_selected()
        elif chosen is a_del:
            self.delete_selected()
        elif chosen is a_new:
            self.make_dir()
        elif chosen is a_newfile:
            self.make_file()
        elif chosen is a_ref:
            self.refresh()
        elif chosen is a_copy:
            from PySide6.QtGui import QGuiApplication
            QGuiApplication.clipboard().setText(
                "\n".join(self.selected_paths() or [self.cwd]))

    # ---- ステータス ----------------------------------------------------------
    def _status(self, msg: str) -> None:
        self.lb_status.setText(msg)
        self._status_timer.start(5000)
        self.status_message.emit(msg)
