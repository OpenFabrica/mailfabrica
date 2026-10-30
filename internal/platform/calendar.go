package platform

import (
	"context"
	_ "embed"
	"encoding/json"
	"fmt"
	"os/exec"
	"runtime"
	"strconv"
	"time"
)

//go:embed calendar.js
var calendarScript string

// CalendarSource 是系统日历元数据。
type CalendarSource struct {
	ID     string `json:"id"`
	Title  string `json:"title"`
	Source string `json:"source"`
	Color  string `json:"color"`
}

// CalendarEvent 与旧版 EventKit 输出字段一致。
type CalendarEvent struct {
	ID       string `json:"id"`
	Calendar string `json:"calendar"`
	Title    string `json:"title"`
	Start    int64  `json:"start"`
	End      int64  `json:"end"`
	AllDay   bool   `json:"allDay"`
	Location string `json:"location"`
}

// CalendarResult 是不依赖 UI 的系统日历查询结果。
type CalendarResult struct {
	Denied    bool             `json:"denied"`
	Calendars []CalendarSource `json:"calendars"`
	Events    []CalendarEvent  `json:"events"`
}

// CalendarSupported 保持非 macOS 平台明确不支持系统日历。
func CalendarSupported() bool { return runtime.GOOS == "darwin" }

// ReadCalendar 使用现有 JXA/EventKit 方式，只有显式加载时才可能请求权限。
func ReadCalendar(ctx context.Context, since, until int64) (CalendarResult, error) {
	out := CalendarResult{Calendars: []CalendarSource{}, Events: []CalendarEvent{}}
	if !CalendarSupported() {
		return out, nil
	}
	ctx, cancel := context.WithTimeout(ctx, 125*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, "/usr/bin/osascript", "-l", "JavaScript", "-e", calendarScript, strconv.FormatInt(since, 10), strconv.FormatInt(until, 10))
	b, e := cmd.Output()
	if e != nil {
		return out, fmt.Errorf("读取系统日历失败或权限请求超时")
	}
	if len(b) > 8<<20 {
		return out, fmt.Errorf("系统日历响应过大")
	}
	if e = json.Unmarshal(b, &out); e != nil {
		return out, fmt.Errorf("系统日历返回无效 JSON")
	}
	if out.Denied {
		return out, fmt.Errorf("Calendar access denied")
	}
	return out, nil
}
