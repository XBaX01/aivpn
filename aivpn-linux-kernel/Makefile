## SPDX-License-Identifier: GPL-2.0

KVER ?= $(shell uname -r)
KDIR ?= /lib/modules/$(KVER)/build
PWD  := $(shell pwd)

# Enforce minimum kernel version 6.1
KVER_MAJOR := $(shell echo $(KVER) | cut -d. -f1)
KVER_MINOR := $(shell echo $(KVER) | cut -d. -f2 | cut -d- -f1)

all: _check_kver
	$(MAKE) -C $(KDIR) M=$(PWD) modules

_check_kver:
	@if [ "$(KVER_MAJOR)" -lt 6 ] || \
	   { [ "$(KVER_MAJOR)" -eq 6 ] && [ "$(KVER_MINOR)" -lt 1 ]; }; then \
		echo "ERROR: Linux 6.1+ required, got $(KVER)" >&2; exit 1; fi

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean

install:
	$(MAKE) -C $(KDIR) M=$(PWD) modules_install
	depmod -a

.PHONY: all _check_kver clean install
