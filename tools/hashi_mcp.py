"""配布用のconsole-mode MCPエントリポイント。"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from hashi.mcp_stdio import main  # noqa: E402

if __name__ == "__main__":
    raise SystemExit(main())
