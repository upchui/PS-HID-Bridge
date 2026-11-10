# PS5 HID Bridge -- every build step runs inside Docker.
# Host requirements: docker, make, git.

SHELL := /bin/bash
.DEFAULT_GOAL := help

include versions.env

UID  := $(shell id -u)
GID  := $(shell id -g)
TTY  := $(shell [ -t 0 ] && echo -t)
JOBS ?= $(shell nproc)
# Qt/qtdeclarative needs ~2 GB RAM per compile job; keep this moderate.
QT_JOBS ?= 6

FW_IMAGE      := pshb-firmware
RUST_IMAGE    := pshb-rust
DESKTOP_IMAGE := pshb-desktop
APPTEST_IMAGE := pshb-apptest

FW_ARGS      := UBUNTU_TAG PICO_SDK_TAG PICOTOOL_TAG ARM_TOOLCHAIN_VERSION ARM_TOOLCHAIN_SHA256
RUST_ARGS    := ALMA_TAG CMAKE_VERSION CMAKE_SHA256 RUSTUP_VERSION RUSTUP_SHA256 RUST_VERSION
DESKTOP_ARGS := $(RUST_ARGS) QT_VERSION QTBASE_SHA256 QTSHADERTOOLS_SHA256 QTDECLARATIVE_SHA256 \
                QTSVG_SHA256 KF_VERSION ECM_SHA256 KIRIGAMI_SHA256 \
                LINUXDEPLOY_TAG LINUXDEPLOY_SHA256 LINUXDEPLOY_QT_TAG LINUXDEPLOY_QT_SHA256 \
                LINUXDEPLOY_APPIMAGE_TAG LINUXDEPLOY_APPIMAGE_SHA256 APPIMAGE_RUNTIME_TAG APPIMAGE_RUNTIME_SHA256

build_args = $(foreach v,$(1),--build-arg $(v)=$($(v)))

FW_BUILD   := .cache/build/firmware
HOST_BUILD := .cache/build/host

DOCKER_RUN = docker run --rm $(TTY) -u $(UID):$(GID) -v $(CURDIR):/work -w /work \
             -e HOME=/work/.cache/home -e JOBS=$(JOBS) $(DOCKER_EXTRA)
RUST_RUN    = $(DOCKER_RUN) -e CARGO_HOME=/work/.cache/cargo -e CARGO_TARGET_DIR=/work/.cache/target/rust $(RUST_IMAGE)
DESKTOP_RUN = $(DOCKER_RUN) -e CARGO_HOME=/work/.cache/cargo -e CARGO_TARGET_DIR=/work/.cache/target/desktop \
              -e APPIMAGE_EXTRACT_AND_RUN=1 $(DESKTOP_IMAGE)
FW_RUN      = $(DOCKER_RUN) $(FW_IMAGE)

.PHONY: help all firmware desktop desktop-cli test test-firmware test-desktop test-appimage \
        test-sim test-protocol test-uf2 sim udp-test fmt submodules check-pins images image-firmware image-rust image-desktop \
        shell-fw shell-desktop clean distclean dirs

help:
	@echo "Targets:"
	@echo "  make firmware      UF2 images            -> out/*.uf2"
	@echo "  make desktop       AppImage (Qt build!)  -> out/PS5-HID-Bridge-x86_64.AppImage"
	@echo "  make desktop-cli   command line tool     -> out/pshb-cli"
	@echo "  make test          unit tests, protocol vectors, simulator"
	@echo "  make sim           board simulator: UDP $(SIM_PORT), web http://localhost:$(SIM_HTTP_PORT)"
	@echo "  make udp-test IP=a.b.c.d ARGS='--plain ping'   test traffic to a board"
	@echo "  make all           everything above"
	@echo "  make submodules    fetch firmware libraries (non-recursive)"
	@echo "  make check-pins    verify submodule tags against versions.env"
	@echo "  make shell-fw | shell-desktop   interactive build shell"
	@echo "  make clean | distclean"

all: firmware desktop-cli desktop test test-uf2 test-appimage

dirs:
	@mkdir -p out .cache/home .cache/cargo

# ---------------------------------------------------------------- images
images: image-firmware image-rust image-desktop

image-firmware:
	docker build $(call build_args,$(FW_ARGS)) -t $(FW_IMAGE) -f docker/firmware.Dockerfile docker

image-rust:
	docker build $(call build_args,$(RUST_ARGS)) --target rust-base -t $(RUST_IMAGE) \
		-f docker/desktop.Dockerfile docker

image-desktop:
	docker build $(call build_args,$(DESKTOP_ARGS)) --build-arg JOBS=$(QT_JOBS) --target desktop \
		-t $(DESKTOP_IMAGE) -f docker/desktop.Dockerfile docker

image-apptest:
	docker build --build-arg ALMA_TAG=$(ALMA_TAG) --target appimage-test \
		-t $(APPTEST_IMAGE) -f docker/desktop.Dockerfile docker

# ---------------------------------------------------------------- sources
submodules:
	git submodule update --init

check-pins:
	@tools/check-pins.sh

# ---------------------------------------------------------------- firmware
firmware: image-firmware check-pins dirs
	$(FW_RUN) bash -c 'cmake -S firmware -B $(FW_BUILD) -G Ninja -DCMAKE_BUILD_TYPE=Release \
		-DPSHB_DEV_PSK=$(DEV_PSK) \
		&& cmake --build $(FW_BUILD) --parallel $(JOBS)'
	cp $(FW_BUILD)/*.uf2 out/
	cp $(FW_BUILD)/ps5-hid-bridge.elf out/
	@ls -l out/*.uf2

# ---------------------------------------------------------------- desktop
desktop-cli: image-rust dirs
	$(RUST_RUN) bash -c 'cd desktop && cargo build --release --locked -p pshb-core --bin pshb-cli'
	cp .cache/target/rust/release/pshb-cli out/

desktop: image-desktop dirs
	$(DESKTOP_RUN) bash desktop/packaging/build-appimage.sh

# ---------------------------------------------------------------- tests
test: test-protocol test-firmware test-sim test-desktop

test-firmware: image-firmware dirs
	$(FW_RUN) bash -c 'cmake -S firmware/host -B $(HOST_BUILD) -G Ninja -DCMAKE_BUILD_TYPE=Debug \
		&& cmake --build $(HOST_BUILD) --parallel $(JOBS) \
		&& ctest --test-dir $(HOST_BUILD) --output-on-failure'

# The real firmware UF2 goes through the network-update writer unchanged.
test-uf2: firmware test-firmware
	$(FW_RUN) env PSHB_TEST_UF2=out/ps5-hid-bridge.uf2 PSHB_TEST_BIN=$(FW_BUILD)/ps5-hid-bridge.bin \
		$(HOST_BUILD)/test_uf2_writer

# Vectors are up to date and the Python reference agrees with them.
test-protocol: image-firmware dirs
	$(FW_RUN) bash -c 'python3 protocol/tools/gen_vectors.py --check && python3 protocol/tools/test_vectors.py'

# Simulator integration tests (the simulator is built by test-firmware),
# including Rust CLI <-> C core interop.
test-sim: test-firmware desktop-cli
	$(FW_RUN) python3 tools/test_sim.py --sim $(HOST_BUILD)/pshb-sim --cli out/pshb-cli

test-desktop: image-rust dirs
	$(RUST_RUN) bash -c 'cd desktop && cargo fmt --all --check \
		&& cargo clippy --locked -p pshb-core --all-targets -- -D warnings \
		&& cargo test --locked -p pshb-core'

fmt: image-rust dirs
	$(RUST_RUN) bash -c 'cd desktop && cargo fmt --all'

# Smoke test of the packaged app on a system without Qt: loads every page
# offscreen (warnings are fatal) and quits.
test-appimage: image-apptest dirs
	$(DOCKER_RUN) -e APPIMAGE_EXTRACT_AND_RUN=1 -e TMPDIR=/tmp -e LANG=C.UTF-8 $(APPTEST_IMAGE) \
		bash -c 'timeout 120 out/PS5-HID-Bridge-x86_64.AppImage --self-test'

# ---------------------------------------------------------------- manual testing
SIM_PORT ?= 45045
SIM_HTTP_PORT ?= 8080
SIM_ARGS ?=

# Simulated board: input on UDP $(SIM_PORT), web interface on http://localhost:$(SIM_HTTP_PORT)
# (settings persist in .cache/sim/flash.bin).
sim: test-firmware
	@mkdir -p .cache/sim
	$(DOCKER_RUN) -i --network host $(FW_IMAGE) $(HOST_BUILD)/pshb-sim --bind 0.0.0.0 \
		--port $(SIM_PORT) --http-port $(SIM_HTTP_PORT) --settings .cache/sim/flash.bin $(SIM_ARGS)

udp-test: image-firmware
	@test -n "$(IP)" || (echo "usage: make udp-test IP=a.b.c.d ARGS='--plain ping'"; exit 2)
	$(DOCKER_RUN) --network host $(FW_IMAGE) python3 tools/udp_test.py $(IP) $(ARGS)

# ---------------------------------------------------------------- misc
shell-fw: image-firmware dirs
	$(DOCKER_RUN) -i $(FW_IMAGE) bash

shell-desktop: image-desktop dirs
	$(DOCKER_RUN) -i -e CARGO_HOME=/work/.cache/cargo -e CARGO_TARGET_DIR=/work/.cache/target/desktop \
		$(DESKTOP_IMAGE) bash

clean:
	rm -rf out .cache/build .cache/target

distclean: clean
	rm -rf .cache
	-docker image rm $(FW_IMAGE) $(RUST_IMAGE) $(DESKTOP_IMAGE)
