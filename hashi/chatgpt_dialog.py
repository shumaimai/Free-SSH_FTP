"""公式OAuthのアカウント/モデル選択。ネットワーク処理はワーカー。"""
import threading

from PySide6.QtCore import QThread, Signal
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QDialogButtonBox,
    QMessageBox,
    QPushButton,
    QVBoxLayout,
)

from . import style
from .chatgpt_oauth import ChatGptProvider


class AccountWorker(QThread):
    result = Signal(object)
    failed = Signal(str)

    def __init__(self, action, parent=None):
        super().__init__(parent)
        self.action, self.cancel = action, threading.Event()

    def run(self):
        try:
            result = self.action(self.cancel)
            if not self.cancel.is_set():
                self.result.emit(result)
        except Exception:
            self.failed.emit("ChatGPT認証処理に失敗しました。ブラウザでの許可、接続状況、アカウントを確認してください")


class ChatGptDialog(QDialog):
    def __init__(self, accounts, parent=None):
        super().__init__(parent)
        self.accounts, self.provider, self.worker = accounts, None, None
        self._reject_pending = False
        self.setWindowTitle("Sign in with ChatGPT")
        self.resize(style.DIALOG_M, 320)
        layout = QVBoxLayout(self)
        layout.addWidget(style.plain_label("ChatGPTプランを利用します。利用枠はChatGPTの他のアプリと共有されます。"))
        self.account = QComboBox()
        layout.addWidget(self.account)
        login = QPushButton("Continue with ChatGPT")
        login.clicked.connect(self._login)
        layout.addWidget(login)
        self.model = QComboBox()
        layout.addWidget(self.model)
        load = QPushButton("このアカウントのモデルを取得")
        load.clicked.connect(self._models)
        layout.addWidget(load)
        logout = QPushButton("このアカウントからサインアウト")
        logout.clicked.connect(self._logout)
        layout.addWidget(logout)
        self.status = style.plain_label("")
        layout.addWidget(self.status)
        self.buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        self.buttons.accepted.connect(self._accept)
        self.buttons.rejected.connect(self.reject)
        layout.addWidget(self.buttons)
        self.controls = [self.account, login, load, logout, self.model]
        self._reload()
        self.account.currentIndexChanged.connect(lambda: self.model.clear())

    def _reload(self, selected=None):
        self.account.clear()
        self.account.addItem("新しいアカウントまたはワークスペース", None)
        for account in self.accounts.list():
            self.account.addItem(account["label"] + ("" if account["connected"] else " · サインアウト済み"), account["id"])
        if selected:
            self.account.setCurrentIndex(self.account.findData(selected))

    def _start(self, action, receiver):
        if self.worker is not None:
            return
        self.status.setText("処理中… キャンセルで停止できます")
        worker = AccountWorker(action, self)
        self.worker = worker
        for widget in self.controls:
            widget.setEnabled(False)
        self.buttons.button(QDialogButtonBox.Ok).setEnabled(False)
        worker.result.connect(receiver)
        worker.failed.connect(self.status.setText)
        worker.finished.connect(self._finished)
        worker.start()

    def _finished(self):
        self.worker = None
        for widget in self.controls:
            widget.setEnabled(True)
        self.buttons.button(QDialogButtonBox.Ok).setEnabled(True)
        if self._reject_pending:
            super().reject()

    def _login(self):
        selected = self.account.currentData()
        self._start(lambda cancel: self.accounts.sign_in(cancel, selected), self._signed_in)

    def _signed_in(self, account_id):
        self._reload(account_id)
        self.status.setText("認証できました。利用可能なモデルを取得してください")

    def _models(self):
        selected = self.account.currentData()
        if selected:
            self._start(lambda cancel: self.accounts.models(selected), self._got_models)

    def _got_models(self, models):
        self.model.clear()
        for model in models:
            self.model.addItem(model["display_name"], model["slug"])
        self.status.setText("モデルを選んでOKを押してください" if models else "利用できるモデルがありません")

    def _logout(self):
        selected = self.account.currentData()
        if selected:
            self._start(lambda cancel: self.accounts.sign_out(selected), self._signed_out)

    def _signed_out(self, revoked):
        self._reload()
        self.model.clear()
        self.status.setText("サインアウトしました" if revoked else
                            "ローカルはサインアウト済み。遠隔の失効は未確認です。ChatGPT設定から接続を解除できます")

    def _accept(self):
        if self.worker is None and self.account.currentData() and self.model.currentData():
            self.provider = ChatGptProvider(self.accounts, self.account.currentData(), self.model.currentData())
            self.accept()
        else:
            QMessageBox.information(self, "ChatGPT", "アカウントと利用可能なモデルを選んでください")

    def reject(self):
        if self.worker is not None and self.worker.isRunning():
            self._reject_pending = True
            self.worker.cancel.set()
            self.status.setText("停止しています…")
            return
        super().reject()

    def closeEvent(self, event):
        if self.worker is not None and self.worker.isRunning():
            self.reject()
            event.ignore()
        else:
            super().closeEvent(event)
