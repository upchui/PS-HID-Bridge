# PS HID Bridge -- every build step runs inside Docker.
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

# Toolchain images. CI overrides the names with registry images and sets
# PREBUILT=1, so the image targets only check that the pulled image exists.
FW_IMAGE      ?= pshb-firmware
RUST_IMAGE    ?= pshb-rust
DESKTOP_IMAGE ?= pshb-desktop
APPTEST_IMAGE ?= pshb-apptest
PREBUILT      ?=
# Extra CMake options for the firmware, e.g. -DPSHB_W5500_FAST_RX=OFF
FW_CMAKE_ARGS ?=
REPO_URL      ?= https://github.com/upchui/PS-HID-Bridge

FW_ARGS      := UBUNTU_TAG PICO_SDK_TAG PICOTOOL_TAG ARM_TOOLCHAIN_VERSION ARM_TOOLCHAIN_SHA256
RUST_ARGS    := ALMA_TAG CMAKE_VERSION CMAKE_SHA256 RUSTUP_VERSION RUSTUP_SHA256 RUST_VERSION
DESKTOP_ARGS := $(RUST_ARGS) QT_VERSION QTBASE_SHA256 QTSHADERTOOLS_SHA256 QTDECLARATIVE_SHA256 \
                QTSVG_SHA256 KF_VERSION ECM_SHA256 KIRIGAMI_SHA256 \
                LINUXDEPLOY_TAG LINUXDEPLOY_SHA256 LINUXDEPLOY_QT_TAG LINUXDEPLOY_QT_SHA256 \
                LINUXDEPLOY_APPIMAGE_TAG LINUXDEPLOY_APPIMAGE_SHA256 APPIMAGE_RUNTIME_TAG APPIMAGE_RUNTIME_SHA256

build_args = $(foreach v,$(1),--build-arg $(v)=$($(v)))

# $(call image,NAME,DOCKER BUILD ARGUMENTS)
ifeq ($(PREBUILT),1)
image = docker image inspect $(1) >/dev/null || { echo "Image $(1) not found (PREBUILT=1)"; exit 1; }
else
image = docker build --label org.opencontainers.image.source=$(REPO_URL) $(2) -t $(1)
endif

FW_BUILD   := .cache/build/firmware
HOST_BUILD := .cache/build/host

# Version MAJOR.MINOR.<commit count> (tools/version.sh) and the commit date,
# passed to every build container (desktop version, AppStream metainfo).
PSHB_VERSION := $(shell tools/version.sh)
PSHB_DATE    := $(shell git log -1 --format=%cs 2>/dev/null)

# Fails (only when a container is started) if the version is unknown.
DOCKER_RUN = $(if $(PSHB_VERSION),,$(error tools/version.sh failed, see above)) \
             docker run --rm $(TTY) -u $(UID):$(GID) -v $(CURDIR):/work -w /work \
             -e HOME=/work/.cache/home -e JOBS=$(JOBS) \
             -e PSHB_VERSION=$(PSHB_VERSION) -e PSHB_DATE=$(PSHB_DATE) \
             $(if $(CARGO_INCREMENTAL),-e CARGO_INCREMENTAL=$(CARGO_INCREMENTAL)) $(DOCKER_EXTRA)
# CLI, tests and AppImage share one target directory, so pshb-core and its
# dependencies are built once. The same RUSTFLAGS everywhere is part of
# that (lld for the GUI comes from desktop/ui/build.rs instead).
CARGO_ENV   = -e CARGO_HOME=/work/.cache/cargo -e CARGO_TARGET_DIR=/work/.cache/target/cargo \
              -e CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=
RUST_RUN    = $(DOCKER_RUN) $(CARGO_ENV) $(RUST_IMAGE)
DESKTOP_RUN = $(DOCKER_RUN) $(CARGO_ENV) -e APPIMAGE_EXTRACT_AND_RUN=1 $(DESKTOP_IMAGE)
FW_RUN      = $(DOCKER_RUN) $(FW_IMAGE)

.PHONY: help all firmware desktop desktop-cli test test-firmware test-desktop test-appimage \
        test-sim test-protocol test-uf2 sim udp-test fmt submodules check-pins check-version images \
        image-firmware image-rust image-desktop image-apptest print-version print-release-tag \
        print-toolchain-tag \
        shell-fw shell-desktop clean distclean dirs

help:
	@echo "Targets:"
	@echo "  make firmware      UF2 images            -> out/*.uf2"
	@echo "  make desktop       AppImage (Qt build!)  -> out/PS-HID-Bridge-x86_64.AppImage"
	@echo "  make desktop-cli   command line tool     -> out/pshb-cli"
	@echo "  make test          unit tests, protocol vectors, simulator"
	@echo "  make sim           board simulator: UDP $(SIM_PORT), web http://localhost:$(SIM_HTTP_PORT)"
	@echo "  make udp-test IP=a.b.c.d ARGS='--plain ping'   test traffic to a board"
	@echo "  make all           everything above"
	@echo "  make submodules    fetch firmware libraries (non-recursive)"
	@echo "  make check-pins    verify submodule tags against versions.env"
	@echo "  make images        build the toolchain images (PREBUILT=1: only check they exist)"
	@echo "  make shell-fw | shell-desktop   interactive build shell"
	@echo "  make clean | distclean"

# Tests (format, lint, unit, simulator) come first, so a mistake shows up
# before the long AppImage build. Parallel builds work too (make -j2 -O).
all: test firmware desktop-cli test-uf2 desktop test-appimage

dirs:
	@mkdir -p out .cache/home .cache/cargo

# ---------------------------------------------------------------- images
images: image-firmware image-rust image-desktop image-apptest

image-firmware:
	$(call image,$(FW_IMAGE),$(call build_args,$(FW_ARGS)) -f docker/firmware.Dockerfile docker)

image-rust:
	$(call image,$(RUST_IMAGE),$(call build_args,$(RUST_ARGS)) --target rust-base \
		-f docker/desktop.Dockerfile docker)

image-desktop:
	$(call image,$(DESKTOP_IMAGE),$(call build_args,$(DESKTOP_ARGS)) --build-arg JOBS=$(QT_JOBS) \
		--target desktop -f docker/desktop.Dockerfile docker)

image-apptest:
	$(call image,$(APPTEST_IMAGE),--build-arg ALMA_TAG=$(ALMA_TAG) --target appimage-test \
		-f docker/desktop.Dockerfile docker)

# Content hash of everything that goes into the images (CI image tag).
print-toolchain-tag:
	@tools/toolchain-tag.sh

# ---------------------------------------------------------------- sources
submodules:
	git submodule update --init

check-pins:
	@tools/check-pins.sh

# The file VERSION holds MAJOR.MINOR; the last part is the commit count.
check-version:
	@tools/version.sh >/dev/null

print-version:
	@tools/version.sh

# Release tag, e.g. v0.9.16: every commit on main gets a higher one.
print-release-tag:
	@v=$$(tools/version.sh) && echo "v$$v"

# ---------------------------------------------------------------- firmware
firmware: image-firmware check-pins dirs
	$(FW_RUN) bash -c 'cmake -S firmware -B $(FW_BUILD) -G Ninja -DCMAKE_BUILD_TYPE=Release \
		-DPSHB_DEV_PSK=$(DEV_PSK) -DPSHB_W5500_FAST_RX=ON $(FW_CMAKE_ARGS) \
		&& cmake --build $(FW_BUILD) --parallel $(JOBS)'
	@# Exactly the images of this build (images.txt), no leftovers in out/.
	rm -f out/*.uf2 out/*.elf
	cd $(FW_BUILD) && cp $$(cat images.txt) ps-hid-bridge.elf $(CURDIR)/out/
	@ls -l out/*.uf2

# ---------------------------------------------------------------- desktop
desktop-cli: image-rust dirs
	$(RUST_RUN) bash -c 'cd desktop && cargo build --release --locked -p pshb-core --bin pshb-cli'
	cp .cache/target/cargo/release/pshb-cli out/

desktop: image-desktop dirs
	$(DESKTOP_RUN) bash desktop/packaging/build-appimage.sh

# ---------------------------------------------------------------- tests
test: check-version test-protocol test-firmware test-sim test-desktop

test-firmware: image-firmware dirs
	$(FW_RUN) bash -c 'cmake -S firmware/host -B $(HOST_BUILD) -G Ninja -DCMAKE_BUILD_TYPE=Debug \
		&& cmake --build $(HOST_BUILD) --parallel $(JOBS) \
		&& ctest --test-dir $(HOST_BUILD) --output-on-failure --timeout 60'

# The real firmware UF2 goes through the network-update writer unchanged.
test-uf2: firmware test-firmware
	$(FW_RUN) env PSHB_TEST_UF2=out/ps-hid-bridge.uf2 PSHB_TEST_BIN=$(FW_BUILD)/ps-hid-bridge.bin \
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
test-appimage: desktop image-apptest dirs
	$(DOCKER_RUN) -e APPIMAGE_EXTRACT_AND_RUN=1 -e TMPDIR=/tmp -e LANG=C.UTF-8 $(APPTEST_IMAGE) \
		bash -c 'timeout 120 out/PS-HID-Bridge-x86_64.AppImage --self-test'

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
	$(DOCKER_RUN) -i $(CARGO_ENV) $(DESKTOP_IMAGE) bash

clean:
	rm -rf out .cache/build .cache/target

distclean: clean
	rm -rf .cache
	-docker image rm $(FW_IMAGE) $(RUST_IMAGE) $(DESKTOP_IMAGE) $(APPTEST_IMAGE)
