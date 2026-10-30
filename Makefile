ifeq ($(shell go env GOOS),darwin)
export CGO_CFLAGS := $(CGO_CFLAGS) -mmacosx-version-min=13.0
export CGO_LDFLAGS := $(CGO_LDFLAGS) -mmacosx-version-min=13.0
endif

VERSION := $(shell cat VERSION)
LDFLAGS := -s -w -X github.com/openfabrica/mailfabrica/internal/engine.Version=$(VERSION)

.PHONY: deps frontend build check demo dev package clean

deps:
	go mod download
	npm ci --prefix frontend

frontend:
	@test -d frontend/node_modules || npm ci --prefix frontend
	npm run build --prefix frontend

build: frontend
	mkdir -p dist
	go build -trimpath -ldflags '$(LDFLAGS)' -o dist/mailfabrica ./cmd/mailfabrica
	go build -trimpath -ldflags '$(LDFLAGS)' -o dist/mailfabrica-cli ./cmd/mailfabrica-cli

check: frontend
	./scripts/check.sh

demo: build
	./dist/mailfabrica --demo --control 7981

dev: frontend
	go run ./cmd/mailfabrica --demo --control 7981

package: build
	python3 scripts/package.py $(PACKAGE_ARGS)

clean:
	rm -rf dist frontend/dist
