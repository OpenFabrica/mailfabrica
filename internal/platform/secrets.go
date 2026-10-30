// Package platform 封装系统凭据等桌面平台能力。
package platform

import (
	"errors"
	"github.com/openfabrica/mailfabrica/internal/store"
	"github.com/zalando/go-keyring"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
)

// Secrets 提供兼容 MailFabrica 服务名的凭据存储。
type Secrets struct {
	mu     sync.Mutex
	root   string
	memory bool
	file   bool
	values map[string]string
}

// NewSecrets 在 demo 中使用内存，显式 file 模式使用权限受限的 JSON。
func NewSecrets(root string, demo bool) *Secrets {
	return &Secrets{root: root, memory: demo, file: strings.EqualFold(os.Getenv("MAILFABRICA_SECRETS"), "file") || runtime.GOOS != "darwin" && runtime.GOOS != "windows", values: map[string]string{}}
}

// Get 返回凭据；不存在与平台错误分别上报，不输出秘密。
func (s *Secrets) Get(id string) (string, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.memory {
		return s.values[id], nil
	}
	if s.file {
		v := map[string]string{}
		if e := store.ReadJSON(filepath.Join(s.root, "secrets.json"), &v); e != nil {
			return "", e
		}
		return v[id], nil
	}
	v, e := keyring.Get("MailFabrica", id)
	if errors.Is(e, keyring.ErrNotFound) {
		return "", nil
	}
	return v, e
}

// Set 写凭据，空字符串表示删除。
func (s *Secrets) Set(id, value string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.memory {
		if value == "" {
			delete(s.values, id)
		} else {
			s.values[id] = value
		}
		return nil
	}
	if s.file {
		v := map[string]string{}
		p := filepath.Join(s.root, "secrets.json")
		if e := store.ReadJSON(p, &v); e != nil {
			return e
		}
		if value == "" {
			delete(v, id)
		} else {
			v[id] = value
		}
		return store.WriteJSON(p, v)
	}
	if value == "" {
		e := keyring.Delete("MailFabrica", id)
		if errors.Is(e, keyring.ErrNotFound) {
			return nil
		}
		return e
	}
	return keyring.Set("MailFabrica", id, value)
}
