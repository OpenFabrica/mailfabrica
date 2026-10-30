// Package invoice 导出发票附件及包含来源信息的清单。
package invoice

import (
	"bytes"
	"encoding/csv"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/store"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"
	"unicode"
)

// Item 汇集导出所需邮件元数据和已解析正文。
type Item struct {
	Meta  mail.Message
	Body  mail.Body
	Error string
}

// Result 返回每封邮件、每个文件的可审阅导出结果。
type Result struct {
	Files  []string
	Failed int
	CSV    string
}

var links = regexp.MustCompile(`https?://[^\s<>"']+`)
var amount = regexp.MustCompile(`(?:金额|价税合计|总计|total|amount)[^0-9]{0,20}([0-9]+(?:\.[0-9]{1,2})?)`)

// safeName 限制文件名字符、长度和路径边界。
func safeName(s string) string {
	var out []rune
	for _, r := range s {
		if !unicode.IsControl(r) && !strings.ContainsRune(`/\:*?"<>|`, r) {
			out = append(out, r)
		}
		if len(out) >= 100 {
			break
		}
	}
	name := strings.Trim(string(out), " .")
	if name == "" {
		return "attachment"
	}
	return name
}

// IsAttachment 识别发票附件，不展开 ZIP 内部内容。
func IsAttachment(a mail.Attachment) bool {
	ext := strings.ToLower(filepath.Ext(a.Name))
	return ext == ".pdf" || ext == ".ofd" || ext == ".xml" || ext == ".zip" || strings.Contains(strings.ToLower(a.Name), "invoice") || strings.Contains(a.Name, "发票")
}

// Export 采用排他文件创建防止覆盖同名文件，生成 UTF-8 BOM 清单。
func Export(dir string, items []Item, progress func(int, int)) (Result, error) {
	result := Result{Files: []string{}}
	if e := os.MkdirAll(dir, 0700); e != nil {
		return result, e
	}
	var buf bytes.Buffer
	buf.WriteString("\xef\xbb\xbf")
	writer := csv.NewWriter(&buf)
	if e := writer.Write([]string{"日期", "发件人", "主题", "金额", "文件", "正文下载链接", "错误"}); e != nil {
		return result, e
	}
	for i, item := range items {
		files := []string{}
		errorText := item.Error
		if errorText == "" {
			for _, a := range item.Body.Attachments {
				if !IsAttachment(a) {
					continue
				}
				prefix := time.Unix(item.Meta.Date, 0).In(time.Local).Format("2006-01-02") + "_" + safeName(item.Meta.From.Name) + "_"
				name := prefix + safeName(filepath.Base(a.Name))
				var file *os.File
				var e error
				for n := 0; n < 10000; n++ {
					candidate := name
					if n > 0 {
						ext := filepath.Ext(name)
						candidate = strings.TrimSuffix(name, ext) + fmt.Sprintf("_%d", n) + ext
					}
					file, e = os.OpenFile(filepath.Join(dir, candidate), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
					if os.IsExist(e) {
						continue
					}
					if e == nil {
						name = candidate
					}
					break
				}
				if e != nil {
					return result, e
				}
				if file == nil {
					return result, fmt.Errorf("同名附件过多")
				}
				if _, e = file.Write(a.Data); e != nil {
					file.Close()
					return result, e
				}
				if e = file.Close(); e != nil {
					return result, e
				}
				files = append(files, name)
				result.Files = append(result.Files, filepath.Join(dir, name))
			}
		} else {
			result.Failed++
		}
		value := ""
		if match := amount.FindStringSubmatch(strings.ToLower(item.Body.Text)); len(match) > 1 {
			value = match[1]
		}
		urls := links.FindAllString(item.Body.Text, 100)
		if e := writer.Write([]string{time.Unix(item.Meta.Date, 0).In(time.Local).Format("2006-01-02"), item.Meta.From.String(), item.Meta.Subject, value, strings.Join(files, "; "), strings.Join(urls, "; "), errorText}); e != nil {
			return result, e
		}
		if progress != nil {
			progress(i+1, len(result.Files))
		}
	}
	writer.Flush()
	if e := writer.Error(); e != nil {
		return result, e
	}
	result.CSV = filepath.Join(dir, "发票清单.csv")
	if e := store.WriteAtomic(result.CSV, buf.Bytes()); e != nil {
		return result, e
	}
	return result, nil
}
