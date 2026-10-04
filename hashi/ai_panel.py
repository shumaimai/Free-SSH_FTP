"""ユーザーが共有対象・実行許可・送信データを管理するAI相談画面。"""
from __future__ import annotations

import threading
import uuid

from PySide6.QtCore import QObject, Qt, QThread, QTimer, Signal
from PySide6.QtGui import QTextCursor
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QHBoxLayout,
    QLabel,
    QListWidget,
    QListWidgetItem,
    QMessageBox,
    QPlainTextEdit,
    QPushButton,
    QTabWidget,
    QVBoxLayout,
    QWidget,
)

from . import style
from .ai_core import AiConversation, TerminalTools, context_snapshot


class ApprovalBridge(QObject):
    requested = Signal(object)

    def ask(self, operation):
        event = threading.Event()
        request = {"operation": operation, "event": event, "approved": False}
        self.requested.emit(request)
        while not event.wait(.1):
            if operation.cancel.is_set():
                return False
        return request["approved"] and not operation.cancel.is_set()


class AiWorker(QThread):
    delta = Signal(str)
    succeeded = Signal(str)
    failed = Signal(str)

    def __init__(self, conversation, question, context, parent=None):
        super().__init__(parent)
        self.conversation, self.question, self.context = conversation, question, context
        self.cancel = threading.Event()

    def run(self):
        try:
            result = self.conversation.ask(self.question, self.context, self.cancel, self.delta.emit)
            self.succeeded.emit(result)
        except Exception as exc:
            self.failed.emit(str(exc))


class AiPanel(QWidget):
    configure_requested = Signal()
    cli_requested = Signal()
    mcp_requested = Signal()

    def __init__(self, broker, parent=None):
        super().__init__(parent)
        self.broker = broker
        self.provider = None
        self.conversation = None
        self.worker = None
        self._workers = []
        self._approvals = []
        self.bridge = ApprovalBridge(self)
        self.bridge.requested.connect(self._approve)
        broker.approval = self.bridge.ask
        layout = QVBoxLayout(self)
        self.connection_status = style.plain_label("AIは未接続です")
        layout.addWidget(self.connection_status)
        connection = QHBoxLayout()
        self.settings_button = QPushButton("AIに接続…")
        self.settings_button.clicked.connect(self.configure_requested)
        connection.addWidget(self.settings_button)
        cli = QPushButton("Claude Codeを開く")
        cli.clicked.connect(self.cli_requested)
        connection.addWidget(cli)
        layout.addLayout(connection)
        self.sharing_status = style.plain_label("端末は共有していません")
        layout.addWidget(self.sharing_status)
        self.pages = QTabWidget()
        layout.addWidget(self.pages, 1)
        conversation = QWidget()
        chat = QVBoxLayout(conversation)
        self.transcript = QPlainTextEdit()
        self.transcript.setReadOnly(True)
        self.transcript.setPlaceholderText("AIに接続して質問してください。端末の状態を見せる場合は「対象端末」で選びます。")
        chat.addWidget(self.transcript, 1)
        self.question = QPlainTextEdit()
        self.question.setPlaceholderText("質問や端末への操作依頼を入力")
        self.question.setMaximumHeight(100)
        chat.addWidget(self.question)
        actions = QHBoxLayout()
        self.send = QPushButton("送信する")
        self.send.clicked.connect(self._send)
        self.send.setEnabled(False)
        actions.addWidget(self.send)
        clear = QPushButton("会話を消去")
        clear.clicked.connect(self.clear)
        actions.addWidget(clear)
        chat.addLayout(actions)
        self.pages.addTab(conversation, "会話")
        targets = QWidget()
        target_layout = QVBoxLayout(targets)
        target_layout.addWidget(style.plain_label("AIとClaude Codeから使う端末を選んでください。未選択の端末は共有しません。"))
        self.sessions = QListWidget()
        target_layout.addWidget(self.sessions, 1)
        controls = QHBoxLayout()
        refresh = QPushButton("端末を更新")
        refresh.clicked.connect(self.refresh_sessions)
        controls.addWidget(refresh)
        self.mode = QComboBox()
        for label, value in (("読むだけ", "consult"), ("操作ごとに確認", "confirm"), ("選択した端末で自動実行", "auto")):
            self.mode.addItem(label, value)
        self.mode.currentIndexChanged.connect(self.stop)
        self.sessions.itemChanged.connect(self._targets_changed)
        target_layout.addWidget(self.mode)
        apply = QPushButton("この端末を共有する")
        apply.clicked.connect(self.apply_sharing)
        controls.addWidget(apply)
        target_layout.addLayout(controls)
        target_layout.addWidget(style.plain_label("共有は15分間です。端末を選び直すか停止すると、実行許可も取り消します。"))
        mcp = QPushButton("ほかのMCPクライアントを接続…")
        mcp.clicked.connect(self.mcp_requested)
        target_layout.addWidget(mcp)
        self.pages.addTab(targets, "対象端末")
        history = QWidget()
        audit_layout = QVBoxLayout(history)
        audit_layout.addWidget(style.plain_label("実行先と結果を表示します。コマンド本文・出力は保存しません。"))
        self.audit_view = QPlainTextEdit()
        self.audit_view.setReadOnly(True)
        audit_layout.addWidget(self.audit_view, 1)
        self.pages.addTab(history, "実行履歴")
        self._last_audit = None
        self.audit_timer = QTimer(self)
        self.audit_timer.setInterval(1000)
        self.audit_timer.timeout.connect(self._refresh_audit)
        self.audit_timer.start()
        stop = QPushButton("停止する・端末の共有を解除")
        stop.clicked.connect(self.stop)
        layout.addWidget(stop)
        for label in self.findChildren(QLabel):
            label.setWordWrap(True)
        self.refresh_sessions()

    def refresh_sessions(self):
        selected = set(self.selected_targets())
        self.sessions.blockSignals(True)
        self.sessions.clear()
        for session in self.broker.registry.list(shareable_only=True):
            kind = "このPCのCMD" if session["kind"] == "local" else "SSH"
            item = QListWidgetItem(f"{session['label']} ({kind})")
            item.setToolTip(f"端末ID: {session['session_id']}\n接続世代: {session['generation']}")
            item.setData(Qt.UserRole, (session["session_id"], session["generation"]))
            item.setFlags(item.flags() | Qt.ItemIsUserCheckable)
            item.setCheckState(Qt.Checked if (session["session_id"], session["generation"]) in selected else Qt.Unchecked)
            self.sessions.addItem(item)
        self.sessions.blockSignals(False)
        if selected != set(self.selected_targets()):
            self.stop()
        self._update_sharing_status()

    def _targets_changed(self):
        self.stop()
        count = len(self.selected_targets())
        self.sharing_status.setText(f"{count}端末を選択中。「この端末を共有する」で適用できます。" if count else "端末は共有していません")

    def _update_sharing_status(self):
        targets = self.broker.list_sessions()
        modes = {"consult": "読むだけ", "confirm": "操作ごとに確認", "auto": "自動実行"}
        state = self.broker.permission_snapshot()
        selected = len(self.selected_targets())
        self.sharing_status.setText(f"{len(targets)}端末を共有中 · {modes[state['mode']]} · 残り{state['seconds'] // 60}分"
                                   if targets else f"{selected}端末を選択中。共有は未適用です。" if selected
                                   else "端末は共有していません")

    def selected_targets(self):
        return [self.sessions.item(i).data(Qt.UserRole) for i in range(self.sessions.count())
                if self.sessions.item(i).checkState() == Qt.Checked]

    def apply_sharing(self):
        try:
            self.broker.configure(self.mode.currentData(), self.selected_targets())
            self._update_sharing_status()
            self.pages.setCurrentIndex(0)
        except (ValueError, PermissionError) as exc:
            self.transcript.appendPlainText(str(exc))

    def _refresh_audit(self):
        current = {(s['session_id'], s['generation']) for s in self.broker.registry.list(shareable_only=True)}
        displayed = {tuple(self.sessions.item(i).data(Qt.UserRole)) for i in range(self.sessions.count())}
        if current != displayed:
            self.refresh_sessions()
        self._update_sharing_status()
        snapshot = self.broker.audit_snapshot()
        if snapshot == self._last_audit:
            return
        self._last_audit = snapshot
        entries = snapshot["finished"][-30:] + snapshot["active"]
        self.audit_view.setPlainText("\n".join(
            f"{entry['actor'].split(':', 1)[0]} · {entry['action']} · "
            f"{entry['session_id']} / {entry['generation']} · {entry['status']} · "
            f"完了 {entry.get('completion', 'unknown')} · 終了コード {entry.get('exit_code')}"
            for entry in entries))

    def set_provider(self, provider):
        if self.worker is not None:
            raise RuntimeError("AI処理が終了してから接続設定を変更してください")
        self.stop()
        self.provider = provider
        names = {"openai": "OpenAI", "anthropic": "Anthropic", "compatible": "互換API", "commandcode": "Command Code"}
        kind = getattr(provider, "kind", "chatgpt")
        self.connection_status.setText(f"{names.get(kind, 'ChatGPT')} · {getattr(provider, 'model', '')}")
        self.settings_button.setText("接続を変更…")
        self.send.setEnabled(True)
        self.conversation = AiConversation(provider, TerminalTools(self.broker, "ai:" + uuid.uuid4().hex))
        self.transcript.clear()

    def _send(self):
        if self.worker is not None:
            return
        if self.provider is None:
            self.transcript.appendPlainText("AI接続設定でプロバイダーを選んでください")
            self.configure_requested.emit()
            return
        question = self.question.toPlainText().strip()
        if not question:
            return
        targets = self.selected_targets()
        try:
            self.broker.configure(self.mode.currentData(), targets)
            snapshot = context_snapshot(self.broker)
        except (ValueError, PermissionError) as exc:
            self.transcript.appendPlainText(str(exc))
            return
        # 一般的な質問には空の端末プレビューを毎回出さない。
        context = ""
        if targets:
            context = self._preview_context(snapshot)
            if context is None:
                self.broker.stop()
                return
        self._update_sharing_status()
        self.transcript.appendPlainText("あなた: " + question)
        self.transcript.appendPlainText("AI: ")
        self._streamed = False
        self.question.clear()
        worker = AiWorker(self.conversation, question, context, self)
        self.worker = worker
        self._workers.append(worker)
        worker.delta.connect(self._delta)
        worker.succeeded.connect(self._succeeded)
        worker.failed.connect(self._failed)
        worker.finished.connect(lambda w=worker: self._finished(w))
        self.send.setEnabled(False)
        self.settings_button.setEnabled(False)
        worker.start()

    def _preview_context(self, snapshot):
        dialog = QDialog(self)
        dialog.setWindowTitle("AIへ送信する内容")
        dialog.resize(style.DIALOG_L, 480)
        v = QVBoxLayout(dialog)
        v.addWidget(style.plain_label("内容は編集できます。秘密情報が残っていないか確認してください。"))
        preview = QPlainTextEdit(snapshot)
        v.addWidget(preview)
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.button(QDialogButtonBox.Ok).setText("この内容を送信する")
        buttons.button(QDialogButtonBox.Cancel).setText("戻る")
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        v.addWidget(buttons)
        if dialog.exec() != QDialog.Accepted:
            return None
        return preview.toPlainText()

    def _delta(self, text):
        self._streamed = True
        self.transcript.moveCursor(QTextCursor.End)
        self.transcript.insertPlainText(text)

    def _succeeded(self, text):
        if not self._streamed:
            self.transcript.moveCursor(QTextCursor.End)
            self.transcript.insertPlainText(text)
        self.transcript.appendPlainText("")

    def _failed(self, text):
        self.transcript.appendPlainText("\n" + text)

    def _finished(self, worker):
        self._workers.remove(worker)
        if self.worker is worker:
            self.worker = None
        self.send.setEnabled(True)
        self.settings_button.setEnabled(True)

    def _approve(self, request):
        operation = request["operation"]
        if operation.cancel.is_set():
            request["event"].set()
            return
        dialog = QMessageBox(self)
        dialog.setWindowTitle("端末操作を確認")
        dialog.setTextFormat(Qt.PlainText)
        dialog.setText(f"端末: {operation.session_id} / 世代 {operation.generation}\n"
                       f"操作: {operation.action}\ncwd: {operation.cwd or '引き継ぎなし'}\n\n{operation.text}")
        dialog.setStandardButtons(QMessageBox.Yes | QMessageBox.No)
        dialog.setDefaultButton(QMessageBox.No)
        self._approvals.append(dialog)
        timer = QTimer(dialog)
        timer.setInterval(100)
        timer.timeout.connect(lambda: dialog.reject() if operation.cancel.is_set() else None)
        timer.start()
        request["approved"] = dialog.exec() == QMessageBox.Yes
        self._approvals.remove(dialog)
        request["event"].set()

    def stop(self):
        self.broker.stop()
        for worker in self._workers:
            worker.cancel.set()
        for dialog in self._approvals:
            dialog.reject()
        self._update_sharing_status()

    def clear(self):
        self.stop()
        if self.worker is None:
            self.transcript.clear()
            if self.provider is not None:
                self.set_provider(self.provider)

    def shutdown(self):
        self.stop()
        return not any(worker.isRunning() for worker in self._workers)
