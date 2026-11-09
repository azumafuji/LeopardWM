#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "python-xlib",
# ]
# ///
"""
WSLg Window Tile Sync Daemon for LeopardWM
Synchronizes X11 application window geometries inside WSL with LeopardWM's tiled RAIL_WINDOW sizes.

WSLg's RemoteApp (msrdc.exe) does not forward host SetWindowPos resizes into Xwayland.
This daemon listens to LeopardWM layout events and synchronizes X11 window geometries
so that WSL2 applications resize to cleanly fill their assigned tiles.
"""

import argparse
import glob
import json
import os
import select
import shutil
import subprocess
import sys
import time

try:
    import Xlib.display
    import Xlib.error
    import Xlib.X
except ImportError:
    print("Error: python-xlib is required. Run via 'uv run --with python-xlib' or install python-xlib.", file=sys.stderr)
    sys.exit(1)

WINDOWS_QUERY_CODE = """
import ctypes, json
from ctypes import wintypes
user32 = ctypes.windll.user32
hdesk = user32.OpenDesktopW("Default", 0, False, 0x01FF)
if hdesk: user32.SetThreadDesktop(hdesk)
res = []
def cb(h, _):
    if not user32.IsWindowVisible(h): return True
    c = ctypes.create_unicode_buffer(128)
    user32.GetClassNameW(h, c, 128)
    if c.value == "RAIL_WINDOW":
        t = ctypes.create_unicode_buffer(512)
        user32.GetWindowTextW(h, t, 512)
        r = wintypes.RECT()
        user32.GetWindowRect(h, ctypes.byref(r))
        res.append({
            "hwnd": h,
            "title": t.value,
            "x": r.left,
            "y": r.top,
            "w": r.right - r.left,
            "h": r.bottom - r.top
        })
    return True
WNDENUMPROC = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
user32.EnumWindows(WNDENUMPROC(cb), 0)
print(json.dumps(res))
"""

def find_python_exe():
    which = shutil.which("python.exe")
    if which:
        return which
    candidates = [
        "/mnt/c/Users/dean/AppData/Local/Programs/Python/Python314-arm64/python.exe",
        "/mnt/c/WINDOWS/py.exe",
    ]
    for c in candidates:
        if os.path.exists(c):
            return c
    globs = glob.glob("/mnt/c/Users/*/AppData/Local/Programs/Python/*/python.exe")
    if globs:
        return globs[0]
    globs_sys = glob.glob("/mnt/c/Program Files/Python*/python.exe")
    if globs_sys:
        return globs_sys[0]
    return "python.exe"

def get_windows_rail_windows(python_bin, verbose=False):
    try:
        out = subprocess.check_output(
            [python_bin, "-c", WINDOWS_QUERY_CODE],
            text=True,
            stderr=subprocess.DEVNULL,
            timeout=3
        ).strip()
        if not out:
            return []
        return json.loads(out)
    except Exception as e:
        if verbose:
            print(f"Error querying Windows RAIL windows via {python_bin}: {e}", file=sys.stderr, flush=True)
        return []

def get_x11_windows(d):
    root = d.screen().root
    x11_windows = []
    try:
        for parent in root.query_tree().children:
            try:
                p_geom = parent.get_geometry()
                if p_geom.width < 50 or p_geom.height < 50:
                    continue
                children = parent.query_tree().children
                for child in children:
                    name_prop = child.get_wm_name()
                    if not name_prop:
                        prop = child.get_full_property(d.get_atom('_NET_WM_NAME'), 0)
                        if prop and prop.value:
                            val = prop.value
                            name_prop = val.decode('utf-8', errors='replace') if isinstance(val, (bytes, bytearray)) else str(val)
                    
                    if not name_prop:
                        prop = child.get_full_property(d.get_atom('WM_CLASS'), 0)
                        if prop:
                            name_prop = "Client"

                    if name_prop:
                        x11_windows.append({
                            "parent": parent,
                            "child": child,
                            "name": name_prop,
                            "geom": child.get_geometry(),
                            "parent_geom": p_geom
                        })
            except (Xlib.error.BadWindow, Xlib.error.BadDrawable):
                continue
    except Exception:
        pass
    return x11_windows

def sync_once(d, python_bin, verbose=True):
    windows_wins = get_windows_rail_windows(python_bin, verbose=verbose)
    if not windows_wins:
        return 0

    x11_wins = get_x11_windows(d)
    if not x11_wins:
        return 0

    windows_wins.sort(key=lambda w: (w["x"], w["y"]))
    x11_wins.sort(key=lambda w: (w["parent_geom"].x, w["parent_geom"].y))

    resized_count = 0
    used_x11 = set()

    for ww in windows_wins:
        w_title = ww["title"]
        target_w = ww["w"]
        target_h = ww["h"]

        clean_win_title = w_title
        if " (" in clean_win_title and clean_win_title.endswith(")"):
            clean_win_title = clean_win_title.rsplit(" (", 1)[0].strip()

        matched = None
        for i, xw in enumerate(x11_wins):
            if i in used_x11:
                continue
            xname = xw["name"]
            if clean_win_title.startswith(xname) or xname.startswith(clean_win_title) or xname == "Client":
                matched = xw
                used_x11.add(i)
                break

        if not matched and len(x11_wins) - len(used_x11) == 1:
            for i, xw in enumerate(x11_wins):
                if i not in used_x11:
                    matched = xw
                    used_x11.add(i)
                    break

        if matched:
            try:
                child = matched["child"]
                parent = matched["parent"]
                prop = child.get_full_property(d.get_atom('_NET_FRAME_EXTENTS'), 0)
                if prop and len(prop.value) >= 4:
                    l, r, t, b = prop.value[:4]
                else:
                    l, r, t, b = 0, 0, 0, 0

                ext_w = l + r
                ext_h = t + b
                child_target_w = max(50, target_w - ext_w)
                child_target_h = max(50, target_h - ext_h)

                c_geom = child.get_geometry()
                p_geom = parent.get_geometry()

                if c_geom.width != child_target_w or c_geom.height != child_target_h:
                    parent.configure(width=target_w, height=target_h)
                    child.configure(width=child_target_w, height=child_target_h)
                    d.sync()
                    resized_count += 1
                    if verbose:
                        ts = time.strftime('%X')
                        print(f"[{ts}] Resized '{matched['name']}': child {c_geom.width}x{c_geom.height} -> {child_target_w}x{child_target_h}, parent {p_geom.width}x{p_geom.height} -> {target_w}x{target_h}", flush=True)
            except (Xlib.error.BadWindow, Xlib.error.BadDrawable):
                pass

    return resized_count

def find_lwm_path():
    candidates = [
        shutil.which("lwm.exe"),
        "/mnt/c/Program Files/LeopardWM/bin/lwm.exe",
        "/mnt/c/Users/dean/Dev/LeopardWM/target/aarch64-pc-windows-msvc/release/lwm.exe",
        "/mnt/c/Users/dean/Dev/LeopardWM/target/release/lwm.exe",
    ]
    for c in candidates:
        if c and os.path.exists(c):
            return c
    return "lwm.exe"

def main():
    parser = argparse.ArgumentParser(description="WSLg Window Tile Sync Daemon for LeopardWM")
    parser.add_argument("-q", "--quiet", action="store_true", help="Suppress resize log messages")
    parser.add_argument("-i", "--interval", type=float, default=1.0, help="Fallback periodic poll interval in seconds (default: 1.0)")
    parser.add_argument("--once", action="store_true", help="Perform one synchronization pass and exit")
    args = parser.parse_args()

    verbose = not args.quiet

    try:
        d = Xlib.display.Display()
    except Exception as e:
        print(f"Failed to connect to X11 display: {e}", file=sys.stderr)
        sys.exit(1)

    python_bin = find_python_exe()
    if verbose:
        print(f"Using Python Windows host binary: {python_bin}", flush=True)

    if args.once:
        count = sync_once(d, python_bin, verbose=verbose)
        if verbose:
            print(f"Synchronized {count} window(s).")
        return

    lwm_bin = find_lwm_path()
    if verbose:
        print("Starting WSLg Tile Sync Daemon for LeopardWM...", flush=True)
        print(f"Using LeopardWM CLI: {lwm_bin}", flush=True)

    # Initial sync pass
    sync_once(d, python_bin, verbose=verbose)

    while True:
        try:
            if verbose:
                print("Subscribing to LeopardWM layout events...", flush=True)
            proc = subprocess.Popen(
                [lwm_bin, "subscribe", "--events", "layout"],
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                bufsize=1
            )

            while proc.poll() is None:
                r, _, _ = select.select([proc.stdout], [], [], args.interval)
                if r:
                    line = proc.stdout.readline()
                    if not line:
                        break
                    sync_once(d, python_bin, verbose=verbose)
                else:
                    sync_once(d, python_bin, verbose=verbose)

            if verbose:
                print("LeopardWM event stream disconnected, retrying in 2 seconds...", flush=True)
            time.sleep(2)
        except KeyboardInterrupt:
            if verbose:
                print("\nExiting.")
            break
        except Exception as e:
            if verbose:
                print(f"Error in sync loop: {e}", flush=True)
            time.sleep(2)

if __name__ == "__main__":
    main()
