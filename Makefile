PREFIX ?= /usr/local
BINDIR = $(PREFIX)/bin
MANDIR = $(PREFIX)/share/man/man1

.PHONY: all build install uninstall clean

all: build

build:
	cargo build --release

install: build
	install -d $(DESTDIR)$(BINDIR)
	install -d $(DESTDIR)$(MANDIR)
	install -m 755 target/release/q $(DESTDIR)$(BINDIR)/q
	install -m 755 target/release/qdaemon $(DESTDIR)$(BINDIR)/qdaemon
	install -m 644 q.1 $(DESTDIR)$(MANDIR)/q.1

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/q
	rm -f $(DESTDIR)$(BINDIR)/qdaemon
	rm -f $(DESTDIR)$(MANDIR)/q.1

clean:
	cargo clean
