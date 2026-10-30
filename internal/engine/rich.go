package engine

import (
	"fmt"
	"html"
	"net/url"
	"reflect"
	"strconv"
	"strings"

	xhtml "golang.org/x/net/html"
)

// InlineStyle 保持按 Unicode 字符计数的行内格式模型。
type InlineStyle struct {
	Bold       bool      `json:"bold"`
	Italic     bool      `json:"italic"`
	Underline  bool      `json:"underline"`
	Strike     bool      `json:"strike"`
	Color      *[3]uint8 `json:"color"`
	Background *[3]uint8 `json:"background"`
	Size       string    `json:"size"`
	Link       *string   `json:"link"`
}

// Span 是半开字符区间，独立于 JavaScript 的 UTF-16 下标。
type Span struct {
	Start int         `json:"start"`
	End   int         `json:"end"`
	Style InlineStyle `json:"style"`
}

// RichDoc 为业务和 agent 提供可序列化的富文本模型。
type RichDoc struct {
	Text   string   `json:"text"`
	Spans  []Span   `json:"spans"`
	Blocks []string `json:"blocks"`
}

// plainStyle 返回与旧版一致的正常字号。
func plainStyle() InlineStyle { return InlineStyle{Size: "normal"} }

// newDoc 初始化每段均为普通段落的富文本。
func newDoc(text string) RichDoc {
	d := RichDoc{Text: text, Spans: []Span{}, Blocks: []string{}}
	for range strings.Split(text, "\n") {
		d.Blocks = append(d.Blocks, "paragraph")
	}
	return d
}

// styleAt 查询某字符的有效样式。
func (d RichDoc) styleAt(index int) InlineStyle {
	for _, span := range d.Spans {
		if index >= span.Start && index < span.End {
			return span.Style
		}
	}
	return plainStyle()
}

// styles 展开字符格式，用于有界编辑后重新归并区间。
func (d RichDoc) styles() []InlineStyle {
	out := make([]InlineStyle, len([]rune(d.Text)))
	for i := range out {
		out[i] = d.styleAt(i)
	}
	return out
}

// setStyles 合并相邻相同格式，不保存默认样式区间。
func (d *RichDoc) setStyles(styles []InlineStyle) {
	d.Spans = []Span{}
	for i, st := range styles {
		if reflect.DeepEqual(st, plainStyle()) {
			continue
		}
		n := len(d.Spans)
		if n > 0 && d.Spans[n-1].End == i && reflect.DeepEqual(d.Spans[n-1].Style, st) {
			d.Spans[n-1].End = i + 1
		} else {
			d.Spans = append(d.Spans, Span{Start: i, End: i + 1, Style: st})
		}
	}
}

// replace 使用共同前后缀保留未变文本的格式，插入样式由命令显式提供。
func (d *RichDoc) replace(text string, insert InlineStyle) {
	old, next := []rune(d.Text), []rune(text)
	a := 0
	for a < len(old) && a < len(next) && old[a] == next[a] {
		a++
	}
	b := 0
	for b < len(old)-a && b < len(next)-a && old[len(old)-1-b] == next[len(next)-1-b] {
		b++
	}
	styles := d.styles()
	newStyles := append([]InlineStyle{}, styles[:a]...)
	for range next[a : len(next)-b] {
		newStyles = append(newStyles, insert)
	}
	newStyles = append(newStyles, styles[len(old)-b:]...)
	oldBlocks := d.Blocks
	d.Text = text
	d.Blocks = make([]string, len(strings.Split(text, "\n")))
	for i := range d.Blocks {
		d.Blocks[i] = "paragraph"
		if i < len(oldBlocks) {
			d.Blocks[i] = oldBlocks[i]
		}
	}
	d.setStyles(newStyles)
}

// parseColor 校验十六进制颜色，不允许 CSS 注入。
func parseColor(v any) (*[3]uint8, error) {
	if v == nil {
		return nil, nil
	}
	s, ok := v.(string)
	if !ok || len(s) != 7 || s[0] != '#' {
		return nil, fmt.Errorf("颜色必须是 #RRGGBB 或 null")
	}
	n, e := strconv.ParseUint(s[1:], 16, 24)
	if e != nil {
		return nil, fmt.Errorf("颜色无效")
	}
	c := [3]uint8{uint8(n >> 16), uint8(n >> 8), uint8(n)}
	return &c, nil
}

// patchStyle 只修改显式给出的格式，并校验链接协议。
func patchStyle(st InlineStyle, p Params) (InlineStyle, error) {
	for key, field := range map[string]*bool{"bold": &st.Bold, "italic": &st.Italic, "underline": &st.Underline, "strike": &st.Strike} {
		if v, ok := p[key]; ok {
			b, ok := v.(bool)
			if !ok {
				return st, fmt.Errorf("%s 必须是布尔值", key)
			}
			*field = b
		}
	}
	for key, field := range map[string]**[3]uint8{"color": &st.Color, "background": &st.Background} {
		if v, ok := p[key]; ok {
			c, e := parseColor(v)
			if e != nil {
				return st, e
			}
			*field = c
		}
	}
	if v, ok := p["size"]; ok {
		size, ok := v.(string)
		if !ok {
			return st, fmt.Errorf("字号无效")
		}
		switch size {
		case "small", "normal", "large", "huge":
			st.Size = size
		default:
			return st, fmt.Errorf("字号无效")
		}
	}
	if v, ok := p["link"]; ok {
		if v == nil {
			st.Link = nil
		} else {
			link, ok := v.(string)
			if !ok || len(link) > 8192 {
				return st, fmt.Errorf("链接无效")
			}
			u, e := url.Parse(link)
			if e != nil || (u.Scheme != "https" && u.Scheme != "http" && u.Scheme != "mailto") {
				return st, fmt.Errorf("仅支持 http/https/mailto 链接")
			}
			st.Link = &link
		}
	}
	return st, nil
}

// HTML 生成仅含可信格式规则的邮件 HTML。
func (d RichDoc) HTML() string {
	var out strings.Builder
	offset := 0
	for lineIndex, line := range strings.Split(d.Text, "\n") {
		block := "paragraph"
		if lineIndex < len(d.Blocks) {
			block = d.Blocks[lineIndex]
		}
		tag := "p"
		switch block {
		case "quote":
			tag = "blockquote"
		case "heading":
			tag = "h2"
		case "bullet", "numbered":
			tag = "li"
		}
		if block == "bullet" {
			out.WriteString("<ul>")
		} else if block == "numbered" {
			out.WriteString("<ol>")
		}
		out.WriteString("<" + tag + " style=\"margin:0 0 8px\">")
		runes := []rune(line)
		for i := 0; i < len(runes); {
			st := d.styleAt(offset + i)
			j := i + 1
			for j < len(runes) && reflect.DeepEqual(st, d.styleAt(offset+j)) {
				j++
			}
			text := html.EscapeString(string(runes[i:j]))
			css := []string{}
			if st.Bold {
				css = append(css, "font-weight:700")
			}
			if st.Italic {
				css = append(css, "font-style:italic")
			}
			dec := []string{}
			if st.Underline {
				dec = append(dec, "underline")
			}
			if st.Strike {
				dec = append(dec, "line-through")
			}
			if len(dec) > 0 {
				css = append(css, "text-decoration:"+strings.Join(dec, " "))
			}
			if st.Color != nil {
				css = append(css, fmt.Sprintf("color:#%02x%02x%02x", st.Color[0], st.Color[1], st.Color[2]))
			}
			if st.Background != nil {
				css = append(css, fmt.Sprintf("background-color:#%02x%02x%02x", st.Background[0], st.Background[1], st.Background[2]))
			}
			switch st.Size {
			case "small":
				css = append(css, "font-size:12px")
			case "large":
				css = append(css, "font-size:18px")
			case "huge":
				css = append(css, "font-size:24px")
			}
			if len(css) > 0 {
				text = "<span style=\"" + strings.Join(css, ";") + "\">" + text + "</span>"
			}
			if st.Link != nil {
				text = "<a href=\"" + html.EscapeString(*st.Link) + "\">" + text + "</a>"
			}
			out.WriteString(text)
			i = j
		}
		if len(runes) == 0 {
			out.WriteString("<br>")
		}
		out.WriteString("</" + tag + ">")
		if block == "bullet" {
			out.WriteString("</ul>")
		} else if block == "numbered" {
			out.WriteString("</ol>")
		}
		offset += len(runes) + 1
	}
	return out.String()
}

// docFromHTML 读取常见邮件标签和行内样式，丢弃活动内容。
func docFromHTML(source string) (RichDoc, error) {
	root, e := xhtml.Parse(strings.NewReader(source))
	if e != nil {
		return RichDoc{}, e
	}
	text := []rune{}
	styles := []InlineStyle{}
	blocks := []string{"paragraph"}
	newline := func(block string) {
		if len(text) > 0 && text[len(text)-1] != '\n' {
			text = append(text, '\n')
			styles = append(styles, plainStyle())
			blocks = append(blocks, block)
		} else if len(blocks) > 0 {
			blocks[len(blocks)-1] = block
		}
	}
	var walk func(*xhtml.Node, InlineStyle, string)
	walk = func(n *xhtml.Node, st InlineStyle, block string) {
		if n.Type == xhtml.ElementNode {
			switch n.Data {
			case "script", "style", "iframe", "object", "head":
				return
			case "b", "strong":
				st.Bold = true
			case "i", "em":
				st.Italic = true
			case "u":
				st.Underline = true
			case "s", "strike", "del":
				st.Strike = true
			case "br":
				text = append(text, '\n')
				styles = append(styles, plainStyle())
				blocks = append(blocks, block)
				return
			case "p", "div":
				newline(block)
			case "blockquote":
				block = "quote"
				newline(block)
			case "h1", "h2", "h3":
				block = "heading"
				newline(block)
			case "ul":
				block = "bullet"
			case "ol":
				block = "numbered"
			case "li":
				newline(block)
			}
			for _, a := range n.Attr {
				if a.Key == "href" && n.Data == "a" {
					if patched, e := patchStyle(st, Params{"link": a.Val}); e == nil {
						st = patched
					}
				}
				if a.Key == "style" {
					for _, pair := range strings.Split(a.Val, ";") {
						kv := strings.SplitN(pair, ":", 2)
						if len(kv) != 2 {
							continue
						}
						key, value := strings.TrimSpace(kv[0]), strings.TrimSpace(kv[1])
						switch key {
						case "font-weight":
							st.Bold = value == "bold" || value == "700" || value == "600"
						case "font-style":
							st.Italic = value == "italic"
						case "text-decoration":
							st.Underline = strings.Contains(value, "underline")
							st.Strike = strings.Contains(value, "line-through")
						case "color":
							if c, e := parseCSSColor(value); e == nil {
								st.Color = c
							}
						case "background-color":
							if c, e := parseCSSColor(value); e == nil {
								st.Background = c
							}
						case "font-size":
							switch value {
							case "12px":
								st.Size = "small"
							case "18px":
								st.Size = "large"
							case "24px":
								st.Size = "huge"
							}
						}
					}
				}
			}
		}
		if n.Type == xhtml.TextNode {
			for _, r := range n.Data {
				text = append(text, r)
				styles = append(styles, st)
			}
		}
		for child := n.FirstChild; child != nil; child = child.NextSibling {
			walk(child, st, block)
		}
		if n.Type == xhtml.ElementNode {
			switch n.Data {
			case "p", "div", "blockquote", "h1", "h2", "h3", "li":
				newline("paragraph")
			}
		}
	}
	walk(root, plainStyle(), "paragraph")
	if len(text) > 0 && text[len(text)-1] == '\n' {
		text = text[:len(text)-1]
		styles = styles[:len(styles)-1]
		if len(blocks) > 1 {
			blocks = blocks[:len(blocks)-1]
		}
	}
	d := RichDoc{Text: string(text), Blocks: blocks}
	d.setStyles(styles)
	return d, nil
}

// parseCSSColor 兼容 HTML 编辑器输出的 rgb、短十六进制和常用颜色名。
func parseCSSColor(value string) (*[3]uint8, error) {
	value = strings.ToLower(strings.TrimSpace(strings.TrimSuffix(value, "!important")))
	named := map[string]string{"black": "#000000", "white": "#ffffff", "red": "#ff0000", "green": "#008000", "blue": "#0000ff", "yellow": "#ffff00", "orange": "#ffa500", "purple": "#800080", "gray": "#808080", "grey": "#808080"}
	if h, ok := named[value]; ok {
		value = h
	}
	if value == "transparent" {
		return nil, nil
	}
	if len(value) == 4 && value[0] == '#' {
		value = fmt.Sprintf("#%c%c%c%c%c%c", value[1], value[1], value[2], value[2], value[3], value[3])
	}
	if strings.HasPrefix(value, "rgb(") || strings.HasPrefix(value, "rgba(") {
		open := strings.IndexByte(value, '(')
		if !strings.HasSuffix(value, ")") {
			return nil, fmt.Errorf("颜色无效")
		}
		parts := strings.FieldsFunc(value[open+1:len(value)-1], func(r rune) bool { return r == ',' || r == ' ' || r == '/' })
		if len(parts) < 3 || len(parts) > 4 {
			return nil, fmt.Errorf("颜色无效")
		}
		if len(parts) == 4 {
			alpha, e := strconv.ParseFloat(strings.TrimSuffix(parts[3], "%"), 64)
			if e != nil {
				return nil, e
			}
			if alpha == 0 {
				return nil, nil
			}
		}
		color := [3]uint8{}
		for i := 0; i < 3; i++ {
			n, e := strconv.ParseFloat(strings.TrimSuffix(parts[i], "%"), 64)
			if e != nil {
				return nil, e
			}
			if strings.HasSuffix(parts[i], "%") {
				n = n * 255 / 100
			}
			if n < 0 {
				n = 0
			}
			if n > 255 {
				n = 255
			}
			color[i] = uint8(n + .5)
		}
		return &color, nil
	}
	return parseColor(value)
}
