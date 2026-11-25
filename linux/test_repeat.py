"""짧은 영상 두 개로 반복 버튼, 키보드 해제, 파일 전환을 검사합니다.

xvfb-run -a python3 linux/test_repeat.py first.mp4 second.mp4
"""
import runpy
import sys
import time
from pathlib import Path
from types import SimpleNamespace

if len(sys.argv) != 3:
    sys.exit("두 개의 짧은 영상 경로를 지정하세요.")
app = runpy.run_path(str(Path(__file__).with_name("gongplayer")), run_name="gongplayer")
Gst, Gtk, Gdk = app["Gst"], app["Gtk"], app["Gdk"]
Gst.init(None)
window = app["Player"]()
audio = Gst.ElementFactory.make("fakesink")
audio.set_property("sync", True)
window.player.set_property("audio-sink", audio)
window.show_all()


def wait_for(condition, timeout=10):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        while Gtk.events_pending():
            Gtk.main_iteration_do(False)
        assert window.path, window.notice.get_text()
        if condition():
            return
        time.sleep(0.02)
    raise AssertionError("예상한 재생 상태에 도달하지 못했습니다.")


for filename in sys.argv[1:]:
    window.open_file(filename)
    assert not window.repeat.get_active()
    wait_for(lambda: window.duration > 0 and window.position() > 0)
    sink = window.player.get_property("video-sink")
    assert sink.get_property("last-sample") is not None
    window.repeat.clicked()
    assert window.repeat.get_active()
    for _ in range(2):
        wait_for(lambda: window.position() > window.duration * 0.7)
        wait_for(lambda: window.position() < window.duration * 0.3)
        assert window.playing
    window.key(window, SimpleNamespace(keyval=Gdk.KEY_l, state=0))
    assert not window.repeat.get_active()
    wait_for(lambda: window.ended)
    assert not window.playing
    window.repeat.clicked()
    wait_for(lambda: window.playing and 0 < window.position() < window.duration / 2)
    print(f"{filename}: 반복 두 번, 해제, 끝에서 다시 시작 확인")
window.player.set_state(Gst.State.NULL)
