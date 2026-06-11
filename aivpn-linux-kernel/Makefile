## SPDX-License-Identifier: GPL-2.0

KVER  ?= $(shell uname -r)
KDIR  ?= /lib/modules/$(KVER)/build
PWD   := $(shell pwd)

obj-m      := aivpn.o
aivpn-y    := src/session_table.o \
               src/crypto_ops.o   \
               src/udp_hook.o     \
               src/tun_inject.o   \
               src/dev.o          \
               src/main.o

EXTRA_CFLAGS := -I$(src)/include

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean

install:
	$(MAKE) -C $(KDIR) M=$(PWD) modules_install
	depmod -a

.PHONY: all clean install
