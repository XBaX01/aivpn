CC      ?= gcc
CFLAGS  := -Wall -Wextra -I../include
TARGET  := user_ioctl_test

all: $(TARGET)

$(TARGET): user_ioctl_test.c
	$(CC) $(CFLAGS) -o $@ $<

test: $(TARGET)
	@echo "NOTE: requires aivpn.ko loaded and root privileges"
	sudo ./$(TARGET)

clean:
	rm -f $(TARGET)

.PHONY: all test clean
