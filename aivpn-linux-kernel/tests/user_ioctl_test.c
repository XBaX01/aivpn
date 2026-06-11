/* user_ioctl_test.c — basic smoke tests for /dev/aivpn ioctls.
 * Requires aivpn.ko loaded. Run as root.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <sys/ioctl.h>
#include <stdint.h>

/* Pull UAPI definitions */
#include "../include/uapi/aivpn.h"

#define DEV "/dev/aivpn"

static int fd;
static int pass_count, fail_count;

#define PASS(name) do { printf("[PASS] %s\n", name); pass_count++; } while(0)
#define FAIL(name, msg) do { printf("[FAIL] %s: %s (errno=%d)\n", name, msg, errno); fail_count++; } while(0)

static void test_open(void)
{
    fd = open(DEV, O_RDWR);
    if (fd < 0) { FAIL("open /dev/aivpn", "open failed"); exit(1); }
    PASS("open /dev/aivpn");
}

static void test_get_version(void)
{
    uint32_t ver = 0;
    if (ioctl(fd, AIVPN_IOC_GET_VERSION, &ver) < 0) {
        FAIL("GET_VERSION", "ioctl failed"); return;
    }
    if (ver != AIVPN_MODULE_API_VERSION) {
        printf("[FAIL] GET_VERSION: expected %u got %u\n", AIVPN_MODULE_API_VERSION, ver);
        fail_count++; return;
    }
    PASS("GET_VERSION");
}

static void test_session_add_remove(void)
{
    struct aivpn_session_add add;
    memset(&add, 0, sizeof(add));
    /* Fill dummy session: session_id = 0x01..0x10 */
    for (int i = 0; i < 16; i++) add.session_id[i] = (uint8_t)(i + 1);
    /* Random-looking keys */
    memset(add.session_key,  0xAB, 32);
    memset(add.tag_secret,   0xCD, 32);
    memset(add.nonce_suffix, 0xEF, 4);
    add.counter_base = 0;
    add.client_ip    = 0x0A000201; /* 10.0.2.1 */
    add.window_ms    = 10000;

    if (ioctl(fd, AIVPN_IOC_SESSION_ADD, &add) < 0) {
        FAIL("SESSION_ADD", "ioctl failed"); return;
    }
    PASS("SESSION_ADD");

    /* Remove by session_id */
    uint8_t del_id[16];
    memcpy(del_id, add.session_id, 16);
    if (ioctl(fd, AIVPN_IOC_SESSION_DEL, del_id) < 0) {
        FAIL("SESSION_DEL", "ioctl failed"); return;
    }
    PASS("SESSION_DEL");
}

static void test_flush(void)
{
    if (ioctl(fd, AIVPN_IOC_FLUSH, NULL) < 0) {
        FAIL("IOC_FLUSH", "ioctl failed"); return;
    }
    PASS("IOC_FLUSH");
}

static void test_invalid_ioctl(void)
{
    /* 0x1234 is not a valid aivpn ioctl — must return EINVAL */
    int ret = ioctl(fd, 0xAE1234, NULL);
    if (ret == 0) { FAIL("INVALID_IOCTL", "expected error, got 0"); return; }
    if (errno != EINVAL) { FAIL("INVALID_IOCTL", "expected EINVAL"); return; }
    PASS("INVALID_IOCTL returns EINVAL");
}

int main(void)
{
    printf("=== aivpn kernel module ioctl smoke tests ===\n");
    test_open();
    test_get_version();
    test_session_add_remove();
    test_flush();
    test_invalid_ioctl();
    close(fd);
    printf("\n%d passed, %d failed\n", pass_count, fail_count);
    return fail_count ? 1 : 0;
}
