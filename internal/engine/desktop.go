package engine

// DesktopState 是宿主需要的轻量状态，隐藏窗口时也能读取，不序列化邮件正文。
type DesktopState struct {
	CloseToTray     bool   `json:"closeToTray"`
	Unread          int    `json:"unread"`
	Language        string `json:"language"`
	UnsavedComposes int    `json:"unsavedComposes"`
	Sending         bool   `json:"sending"`
}

// DesktopState 从已同步的收件箱计数汇总徽标，不计草稿、归档和垃圾邮件。
func (s *Session) DesktopState() DesktopState {
	s.mu.Lock()
	defer s.mu.Unlock()
	state := DesktopState{CloseToTray: boolean(s.prefs, "closeToTray", false), Language: str(s.prefs, "language")}
	for _, account := range s.Store.Accounts {
		folders := s.Store.Folders[account.ID]
		for _, folder := range folders {
			if folder.Role == "inbox" {
				state.Unread += int(folder.Unread)
			}
		}
	}
	for _, compose := range s.composes {
		if compose.Dirty {
			state.UnsavedComposes++
		}
		if compose.Sending {
			state.Sending = true
		}
	}
	return state
}
