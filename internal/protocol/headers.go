package protocol

import (
	"mime"
	"strings"

	"github.com/emersion/go-message/charset"
	"github.com/openfabrica/mailfabrica/internal/mail"
)

// DecodeHeader 解码 RFC 2047 的 B/Q 编码和旧字符集；损坏编码保持原值，避免丢失邮件头。
func DecodeHeader(value string) string {
	if !strings.Contains(value, "=?") {
		return value
	}
	decoder := mime.WordDecoder{CharsetReader: charset.Reader}
	decoded, err := decoder.DecodeHeader(value)
	if err != nil {
		return value
	}
	return decoded
}

// DecodeMetadata 为远端 ENVELOPE 和旧缓存提供统一显示语义，不修改邮件原文。
func DecodeMetadata(value mail.Message) mail.Message {
	value.Subject = DecodeHeader(value.Subject)
	value.From.Name = DecodeHeader(value.From.Name)
	value.To = decodeAddresses(value.To)
	value.CC = decodeAddresses(value.CC)
	return value
}

// decodeAddresses 复制地址数组后解码姓名，不改变共享缓存中的切片。
func decodeAddresses(values []mail.Address) []mail.Address {
	result := append([]mail.Address{}, values...)
	for i := range result {
		result[i].Name = DecodeHeader(result[i].Name)
	}
	return result
}
