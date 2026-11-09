# WSLg Window Tile Sync Helper for LeopardWM

WSLg runs Linux GUI applications under an internal Weston/Xwayland compositor and projects them into Windows via RemoteApp (`msrdc.exe`, window class `RAIL_WINDOW`).

When LeopardWM tiles and resizes a `RAIL_WINDOW` on Windows via `SetWindowPos`, `msrdc.exe` resizes its host proxy window rect but does not propagate the resize event over RDP back to Xwayland. As a result, the internal X11 application buffer does not resize to fill the newly assigned tile.

`wslg-tile-sync.py` runs inside your WSL2 distro to bridge this gap:
1. It subscribes to LeopardWM's IPC layout events (`lwm subscribe --events layout`).
2. Whenever layout changes (or a new window appears), it reads the host `RAIL_WINDOW` tile rects and maps them to X11 application windows (`DISPLAY=:0`).
3. It configures the matching X11 client and frame windows via Xlib (`_NET_FRAME_EXTENTS`), allowing applications like Chromium, VS Code, or terminals to immediately redraw at full tile resolution.

## Requirements

- Python 3.10+ in your WSL distro.
- [`uv`](https://docs.astral.sh/uv/) (recommended; script has inline PEP 723 metadata to auto-install `python-xlib`), or `python3-xlib` installed via package manager.

## Usage

### Run once:
```bash
./tools/wsl/wslg-tile-sync.py --once
```

### Run interactively in the foreground:
```bash
./tools/wsl/wslg-tile-sync.py
```

### Install as a systemd user service (auto-starts with WSL):
```bash
mkdir -p ~/.local/bin ~/.config/systemd/user
cp tools/wsl/wslg-tile-sync.py ~/.local/bin/wslg-tile-sync
chmod +x ~/.local/bin/wslg-tile-sync

cat <<'EOF' > ~/.config/systemd/user/wslg-tile-sync.service
[Unit]
Description=LeopardWM WSLg Window Tile Sync Daemon
After=default.target

[Service]
Type=simple
Environment=DISPLAY=:0
Environment=WAYLAND_DISPLAY=wayland-0
Environment=PYTHONUNBUFFERED=1
ExecStart=/home/%u/.local/bin/wslg-tile-sync -q
Restart=always
RestartSec=3

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
systemctl --user enable --now wslg-tile-sync
```
