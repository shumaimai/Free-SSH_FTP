"""ユーザーが共有対象・実行許可・送信データを管理するAI相談画面。"""
from __future__ import annotations

import threading
import uuid

from PySide6.QtCore import QObject, Qt, QThread, QTimer, Signal
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QHBoxLayout,
    QListWidget,
    QListWidgetItem,
    QMessageBox,
    QPlainTextEdit,
    QPushButton,
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
        layout.addWidget(style.plain_label("共有する端末と実行モードを選んでください"))
        self.sessions = QListWidget()
        self.sessions.setMaximumHeight(150)
        layout.addWidget(self.sessions)
        controls = QHBoxLayout()
        refresh = QPushButton("端末を更新")
        refresh.clicked.connect(self.refresh_sessions)
        controls.addWidget(refresh)
        self.mode = QComboBox()
        for label, value in (("相談のみ", "consult"), ("毎回確認", "confirm"), ("指定端末で自動実行", "auto")):
            self.mode.addItem(label, value)
        self.mode.currentIndexChanged.connect(self.stop)
        self.sessions.itemChanged.connect(self.stop)
        controls.addWidget(self.mode)
        apply = QPushButton("AI/MCP共有を適用")
        apply.clicked.connect(self.apply_sharing)
        controls.addWidget(apply)
        layout.addLayout(controls)
        self.transcript = QPlainTextEdit()
        self.transcript.setReadOnly(True)
        layout.addWidget(self.transcript, 1)
        self.question = QPlainTextEdit()
        self.question.setPlaceholderText("端末の状況について相談する、または操作を依頼する")
        self.question.setMaximumHeight(100)
        layout.addWidget(self.question)
        buttons = QHBoxLayout()
        self.send = QPushButton("送信内容を確認")
        self.send.clicked.connect(self._send)
        buttons.addWidget(self.send)
        stop = QPushButton("停止・許可を取り消す")
        stop.clicked.connect(self.stop)
        buttons.addWidget(stop)
        self.settings_button = QPushButton("AI接続設定")
        self.settings_button.clicked.connect(self.configure_requested)
        buttons.addWidget(self.settings_button)
        clear = QPushButton("会話を消去")
        clear.clicked.connect(self.clear)
        buttons.addWidget(clear)
        layout.addLayout(buttons)
        self.refresh_sessions()

    def refresh_sessions(self):
        self.broker.stop()
        self.sessions.clear()
        for session in self.broker.registry.list(shareable_only=True):
            item = QListWidgetItem(f"{session['label']} · {session['kind']} · {session['session_id'][:8]} / {session['generation']}")
            item.setData(Qt.UserRole, (session["session_id"], session["generation"]))
            item.setFlags(item.flags() | Qt.ItemIsUserCheckable)
            item.setCheckState(Qt.Unchecked)
            self.sessions.addItem(item)

    def selected_targets(self):
        return [self.sessions.item(i).data(Qt.UserRole) for i in range(self.sessions.count())
                if self.sessions.item(i).checkState() == Qt.Checked]

    def apply_sharing(self):
        try:
            self.broker.configure(self.mode.currentData(), self.selected_targets())
            self.transcript.appendPlainText("選択した端末をAI/MCPへ15分間共有しました。停止で取り消せます。")
        except (ValueError, PermissionError) as exc:
            self.transcript.appendPlainText(str(exc))

    def set_provider(self, provider):
        if self.worker is not None:
            raise RuntimeError("AI処理が終了してから接続設定を変更してください")
        self.stop()
        self.provider = provider
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
        try:
            self.broker.configure(self.mode.currentData(), self.selected_targets())
            snapshot = context_snapshot(self.broker)
        except (ValueError, PermissionError) as exc:
            self.transcript.appendPlainText(str(exc))
            return
        dialog = QDialog(self)
        dialog.setWindowTitle("AIへ送信する内容")
        dialog.resize(style.DIALOG_L, 480)
        v = QVBoxLayout(dialog)
        v.addWidget(style.plain_label("内容は編集できます。秘密情報が残っていないか確認してください。"))
        preview = QPlainTextEdit(snapshot)
        v.addWidget(preview)
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(dialog.accept)
        buttons.rejected.connect(dialog.reject)
        v.addWidget(buttons)
        if dialog.exec() != QDialog.Accepted:
            self.broker.stop()
            return
        self.transcript.appendPlainText("あなた: " + question)
        self.transcript.appendPlainText("AI: ")
        self._streamed = False
        self.question.clear()
        worker = AiWorker(self.conversation, question, preview.toPlainText(), self)
        self.worker = worker
        self._workers.append(worker)
        worker.delta.connect(self._delta)
        worker.succeeded.connect(self._succeeded)
        worker.failed.connect(self._failed)
        worker.finished.connect(lambda w=worker: self._finished(w))
        self.send.setEnabled(False)
        self.settings_button.setEnabled(False)
        worker.start()

    def _delta(self, text):
        self._streamed = True
        self.transcript.insertPlainText(text)

    def _succeeded(self, text):
        if not self._streamed:
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

    def clear(self):
        self.stop()
        if self.worker is None:
            self.transcript.clear()
            if self.provider is not None:
                self.set_provider(self.provider)

    def shutdown(self):
        self.stop()
        return not any(worker.isRunning() for worker in self._workers)
