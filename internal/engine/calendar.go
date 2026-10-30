package engine

import (
	"context"
	"fmt"
	lunar "github.com/6tail/lunar-go/calendar"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/platform"
	"time"
)

// CalendarState 记录当前系统日历查询的状态与数据。
type CalendarState struct {
	Supported bool                      `json:"supported"`
	Busy      bool                      `json:"busy"`
	Error     *string                   `json:"error"`
	Calendars []platform.CalendarSource `json:"calendars"`
	Events    []platform.CalendarEvent  `json:"events"`
	Since     string                    `json:"since"`
	Until     string                    `json:"until"`
}

// dateRange 解析本地日期闭区间，限制最多 400 天。
func dateRange(p Params, optional bool) (time.Time, time.Time, error) {
	since, until := str(p, "since"), str(p, "until")
	var a, b time.Time
	var e error
	if since == "" && optional {
		a = time.Unix(0, 0)
	} else {
		a, e = time.ParseInLocation("2006-01-02", since, time.Local)
		if e != nil {
			return a, b, fmt.Errorf("since 必须为 YYYY-MM-DD")
		}
	}
	if until == "" && optional {
		b = time.Date(2200, 1, 1, 0, 0, 0, 0, time.Local)
	} else {
		b, e = time.ParseInLocation("2006-01-02", until, time.Local)
		if e != nil {
			return a, b, fmt.Errorf("until 必须为 YYYY-MM-DD")
		}
		b = b.AddDate(0, 0, 1)
	}
	if b.Before(a) || b.Equal(a) {
		return a, b, fmt.Errorf("日期范围无效")
	}
	if !optional && b.Sub(a) > 400*24*time.Hour+time.Hour {
		return a, b, fmt.Errorf("最多查询 400 天")
	}
	return a, b, nil
}

// includesFilter 对显式数组执行过滤，空数组表示不显示任何来源。
func includesFilter(p Params, name, id string) (bool, error) {
	v, ok := p[name]
	if !ok || v == nil {
		return true, nil
	}
	list, ok := v.([]any)
	if !ok {
		return false, fmt.Errorf("%s 必须是字符串数组", name)
	}
	included := false
	for _, item := range list {
		s, ok := item.(string)
		if !ok {
			return false, fmt.Errorf("%s 必须是字符串数组", name)
		}
		included = included || s == id
	}
	return included, nil
}

// calendarCommand 提供日历授权加载、按日邮件和农历。
func (s *Session) calendarCommand(id string, p Params) (any, error) {
	s.calendar.Supported = !s.demo && platform.CalendarSupported()
	if s.calendar.Calendars == nil {
		s.calendar.Calendars = []platform.CalendarSource{}
	}
	if s.calendar.Events == nil {
		s.calendar.Events = []platform.CalendarEvent{}
	}
	if id == "calendar.status" {
		return s.calendar, nil
	}
	a, b, e := dateRange(p, false)
	if e != nil {
		return nil, e
	}
	if id == "calendar.load" {
		if s.calendar.Busy {
			return s.calendar, nil
		}
		if !boolean(p, "force", false) && s.calendar.Since == str(p, "since") && s.calendar.Until == str(p, "until") {
			return s.calendar, nil
		}
		s.calendar.Since = str(p, "since")
		s.calendar.Until = str(p, "until")
		if !s.calendar.Supported {
			return s.calendar, nil
		}
		s.calendar.Busy = true
		s.calendar.Error = nil
		e = s.launch("calendar", func(ctx context.Context) (any, error) {
			result, err := platform.ReadCalendar(ctx, a.Unix(), b.Unix())
			if err != nil {
				s.mu.Lock()
				s.calendar.Busy = false
				msg := err.Error()
				s.calendar.Error = &msg
				s.mu.Unlock()
			}
			return result, err
		}, func(v any) error {
			s.calendar.Busy = false
			r, ok := v.(platform.CalendarResult)
			if !ok {
				return fmt.Errorf("日历结果无效")
			}
			s.calendar.Calendars = r.Calendars
			s.calendar.Events = r.Events
			return nil
		})
		return s.calendar, e
	}
	messages, e := s.allMessages()
	if e != nil {
		return nil, e
	}
	days := []Params{}
	for day := a; day.Before(b); day = day.AddDate(0, 0, 1) {
		events := []platform.CalendarEvent{}
		mails := []Params{}
		count := 0
		next := day.AddDate(0, 0, 1)
		for _, event := range s.calendar.Events {
			allowed, e := includesFilter(p, "calendars", event.Calendar)
			if e != nil {
				return nil, e
			}
			if allowed && event.Start < next.Unix() && event.End >= day.Unix() {
				events = append(events, event)
			}
		}
		for _, m := range messages {
			allowed, e := includesFilter(p, "accounts", m.Key.Account)
			if e != nil {
				return nil, e
			}
			role := mail.FolderRole(m.Key.Folder, nil)
			for _, f := range s.Store.Folders[m.Key.Account] {
				if f.Path == m.Key.Folder {
					role = f.Role
				}
			}
			if allowed && role != "sent" && role != "drafts" && role != "trash" && role != "junk" && m.Date >= day.Unix() && m.Date < next.Unix() {
				count++
				if len(mails) < 200 {
					mails = append(mails, Params{"key": m.Key.String(), "account": m.Key.Account, "subject": m.Subject, "from": m.From, "date": m.Date, "unread": !m.Flags.Seen, "flagged": m.Flags.Flagged})
				}
			}
		}
		label := ""
		if day.Year() >= 1900 && day.Year() <= 2100 {
			l := lunar.NewSolarFromDate(day).GetLunar()
			label = l.GetDayInChinese()
			if l.GetDay() == 1 {
				label = l.GetMonthInChinese() + "月"
			}
		}
		days = append(days, Params{"date": day.Format("2006-01-02"), "lunar": label, "events": events, "messages": mails, "messageCount": count})
	}
	return Params{"supported": s.calendar.Supported, "busy": s.calendar.Busy, "error": s.calendar.Error, "calendars": s.calendar.Calendars, "since": s.calendar.Since, "until": s.calendar.Until, "events": len(s.calendar.Events), "days": days}, nil
}
