from PySide6.QtWidgets import QMessageBox

from hashi.mainwindow import SessionTab


class _SecretContext:
    def get_sudo_password(self):
        return "saved-password"


class _Terminal:
    def __init__(self):
        self.passwords = []

    def send_password(self, password):
        self.passwords.append(password)


class _SessionTab:
    settings = {"sudo_autofill": True}
    secret_ctx = _SecretContext()

    def __init__(self):
        self._last_autofill_ts = 0
        self.terminal = _Terminal()
        self.messages = []

    def _flash(self, message, warn=False):
        self.messages.append((message, warn))


def test_sudo_prompt_does_not_send_password_without_confirmation(monkeypatch):
    tab = _SessionTab()
    monkeypatch.setattr(QMessageBox, "question", lambda *args: QMessageBox.No)

    SessionTab._on_password_prompt(tab, "sudo")

    assert tab.terminal.passwords == []


def test_sudo_prompt_sends_password_after_confirmation(monkeypatch):
    tab = _SessionTab()
    monkeypatch.setattr(QMessageBox, "question", lambda *args: QMessageBox.Yes)

    SessionTab._on_password_prompt(tab, "sudo")

    assert tab.terminal.passwords == ["saved-password"]
