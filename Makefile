# ryukaze on systemd distributions other than NixOS (see readme.md):
# `make`, then `sudo make install`. In /usr/local unless told; packages set
# PREFIX=/usr and DESTDIR. USERS, a list of user names, lets them change
# the EC, as the NixOS module's `users` does; everyone may read it.

PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
LIBDIR ?= $(PREFIX)/lib
DATADIR ?= $(PREFIX)/share
# D-Bus reads the system bus's policies in /usr/share and /etc only.
DBUSDIR ?= $(if $(filter /usr,$(PREFIX)),$(DATADIR)/dbus-1/system.d,/etc/dbus-1/system.d)
USERS ?=
CARGO ?= cargo

POLICY = io.github.kontonkara.ryukaze

all:
	$(CARGO) build --release --locked

# Built already: as root, cargo would build again, into root's files.
install:
	install -Dm755 target/release/ryukazed $(DESTDIR)$(BINDIR)/ryukazed
	install -Dm755 target/release/ryukaze $(DESTDIR)$(BINDIR)/ryukaze
	install -d $(DESTDIR)$(LIBDIR)/systemd/system $(DESTDIR)$(LIBDIR)/systemd/user
	sed 's|@bindir@|$(BINDIR)|g' data/ryukazed.service.in \
		> $(DESTDIR)$(LIBDIR)/systemd/system/ryukazed.service
	sed 's|@bindir@|$(BINDIR)|g' data/ryukaze.service.in \
		> $(DESTDIR)$(LIBDIR)/systemd/user/ryukaze.service
	sed 's|@bindir@|$(BINDIR)|g' data/ryukaze-osd.service.in \
		> $(DESTDIR)$(LIBDIR)/systemd/user/ryukaze-osd.service
	install -Dm644 data/modules-load.conf $(DESTDIR)$(LIBDIR)/modules-load.d/ryukaze.conf
	install -Dm644 data/modprobe.conf $(DESTDIR)$(LIBDIR)/modprobe.d/ryukaze.conf
	install -Dm644 data/$(POLICY).conf $(DESTDIR)$(DBUSDIR)/$(POLICY).conf
	install -Dm644 data/ryukaze.desktop $(DESTDIR)$(DATADIR)/applications/ryukaze.desktop
	install -Dm644 data/ryukaze.svg $(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/ryukaze.svg
ifneq ($(strip $(USERS)),)
	{ \
		echo '<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"'; \
		echo ' "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">'; \
		echo '<busconfig>'; \
		for user in $(USERS); do \
			echo "  <policy user=\"$$user\">"; \
			echo '    <allow send_destination="$(POLICY)"/>'; \
			echo '  </policy>'; \
		done; \
		echo '</busconfig>'; \
	} > $(DESTDIR)$(DBUSDIR)/$(POLICY).users.conf
endif

# Not /etc/ryukaze or /var/lib/ryukaze: the config, and what the window kept.
uninstall:
	rm -f $(DESTDIR)$(BINDIR)/ryukazed $(DESTDIR)$(BINDIR)/ryukaze \
		$(DESTDIR)$(LIBDIR)/systemd/system/ryukazed.service \
		$(DESTDIR)$(LIBDIR)/systemd/user/ryukaze.service \
		$(DESTDIR)$(LIBDIR)/systemd/user/ryukaze-osd.service \
		$(DESTDIR)$(LIBDIR)/modules-load.d/ryukaze.conf \
		$(DESTDIR)$(LIBDIR)/modprobe.d/ryukaze.conf \
		$(DESTDIR)$(DBUSDIR)/$(POLICY).conf $(DESTDIR)$(DBUSDIR)/$(POLICY).users.conf \
		$(DESTDIR)$(DATADIR)/applications/ryukaze.desktop \
		$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/ryukaze.svg

.PHONY: all install uninstall
