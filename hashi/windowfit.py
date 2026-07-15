"""トップレベルウィンドウを画面の作業領域に収める(Issue #63)。

Windows 11 で、希望サイズが画面より大きいとタイトルバーが画面上端の外へ
突き出し、ウィンドウを掴んで移動できなくなる。表示前にサイズを作業領域へ
クランプし、フレーム(タイトルバー)ぶんの余白を確保して配置する。
"""
from PySide6.QtGui import QGuiApplication

# タイトルバー + ウィンドウ枠の見込み高さ。表示前は正確なフレーム寸法が
# 取れない(Qt はネイティブハンドル生成後にしか分からない)ため定数で確保する。
FRAME_ALLOWANCE = 48


def fit_to_screen(win, width: int, height: int) -> None:
    """win を width x height を上限に作業領域へ収め、中央に配置する。

    - 高さはフレームぶん(FRAME_ALLOWANCE)を差し引いてクランプし、
      タイトルバーが必ず作業領域内に入るようにする。
    - 位置は作業領域内で中央寄せ。マルチモニタの負座標(メイン画面の左に
      サブ画面がある構成)でも avail.x()/y() 起点なので壊れない。
    """
    screen = win.screen() or QGuiApplication.primaryScreen()
    if screen is None:
        win.resize(width, height)
        return
    avail = screen.availableGeometry()
    w = min(width, avail.width())
    h = min(height, max(1, avail.height() - FRAME_ALLOWANCE))
    win.resize(w, h)
    x = avail.x() + max(0, (avail.width() - w) // 2)
    y = avail.y() + max(0, (avail.height() - FRAME_ALLOWANCE - h) // 2)
    win.move(x, y)
