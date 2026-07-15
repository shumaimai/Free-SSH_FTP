"""ウィンドウの画面内クランプ(Issue #63)のテスト。"""
from PySide6.QtGui import QGuiApplication
from PySide6.QtWidgets import QWidget

from hashi.windowfit import FRAME_ALLOWANCE, fit_to_screen


def _avail():
    return QGuiApplication.primaryScreen().availableGeometry()


def test_oversized_window_is_clamped_into_screen(qapp):
    avail = _avail()
    w = QWidget()
    fit_to_screen(w, avail.width() + 2000, avail.height() + 2000)
    # サイズは作業領域以下(高さはタイトルバーぶんの余白込み)
    assert w.width() <= avail.width()
    assert w.height() <= avail.height() - FRAME_ALLOWANCE
    # 位置(フレーム基準)は作業領域の内側 → タイトルバーが上へ突き出さない
    assert w.x() >= avail.x()
    assert w.y() >= avail.y()
    w.deleteLater()


def test_small_window_keeps_requested_size_and_centers(qapp):
    avail = _avail()
    w = QWidget()
    fit_to_screen(w, 300, 200)
    assert (w.width(), w.height()) == (300, 200)
    assert avail.x() <= w.x() <= avail.x() + avail.width() - 300
    assert w.y() >= avail.y()
    w.deleteLater()
