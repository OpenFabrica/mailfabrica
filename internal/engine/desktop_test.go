package engine

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

// TestDesktopUnreadBadge 验证数字跟随收件箱未读状态，忽略其他目录，偏好通过命令持久化。
func TestDesktopUnreadBadge(t *testing.T) {
	s := testSession(t)
	initial := s.DesktopState()
	if initial.CloseToTray || initial.Unread != 5 {
		t.Fatalf("initial %+v", initial)
	}
	s.mu.Lock()
	folders := s.Store.Folders["demo-mailfabrica-dev"]
	for i := range folders {
		if folders[i].Role == "junk" || folders[i].Role == "archive" {
			folders[i].Unread = 200
		}
	}
	s.Store.Folders["demo-mailfabrica-dev"] = folders
	s.mu.Unlock()
	if s.DesktopState().Unread != 5 {
		t.Fatal("counted non-inbox folders")
	}
	executeTest(t, s, "view.select", Params{"key": "demo-mailfabrica-dev/1/INBOX"})
	if s.DesktopState().Unread != 4 {
		t.Fatal("read message did not decrease badge")
	}
	executeTest(t, s, "message.markRead", Params{"read": false})
	if s.DesktopState().Unread != 5 {
		t.Fatal("unread message did not increase badge")
	}
	executeTest(t, s, "app.prefs", Params{"closeToTray": true})
	data, err := os.ReadFile(filepath.Join(s.Store.Root, "prefs.json"))
	if err != nil {
		t.Fatal(err)
	}
	var prefs Params
	if err = json.Unmarshal(data, &prefs); err != nil {
		t.Fatal(err)
	}
	if prefs["closeToTray"] != true || !s.DesktopState().CloseToTray {
		t.Fatal("tray preference was not persisted")
	}
	if _, err = s.Execute("app.prefs", marshalParams(Params{"closeToTray": "true"})); err == nil {
		t.Fatal("accepted non-boolean tray preference")
	}
	var compose struct{ ID uint64 }
	if err = json.Unmarshal(executeTest(t, s, "compose.new", Params{}), &compose); err != nil {
		t.Fatal(err)
	}
	executeTest(t, s, "compose.set", Params{"id": float64(compose.ID), "body": "未保存的内容"})
	if s.DesktopState().UnsavedComposes != 1 {
		t.Fatal("unsaved compose missing from quit guard")
	}
	executeTest(t, s, "compose.discard", Params{"id": float64(compose.ID)})
	if s.DesktopState().UnsavedComposes != 0 {
		t.Fatal("closed compose still blocks quit")
	}
}
