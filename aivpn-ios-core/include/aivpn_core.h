#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Callback invoked from Rust when the VPN tunnel is ready (handshake complete).
/// @param host  NUL-terminated server hostname (valid only for the duration of the callback).
/// @param ctx   Opaque pointer passed back from aivpn_run_tunnel.
typedef void (*aivpn_ready_callback_t)(const char *host, void *ctx);

/// Run a full VPN tunnel session (blocks until done).
///
/// @param tun_fd       AF_UNIX SOCK_DGRAM socketpair fd (Rust reads/writes IP packets).
///                     Rust dups it internally — caller keeps ownership.
/// @param server_host  NUL-terminated server hostname or IP address.
/// @param server_port  UDP port.
/// @param server_key   32-byte server X25519 public key.
/// @param psk          32-byte pre-shared key, or NULL.
/// @param on_ready     Callback invoked once the handshake succeeds (may be NULL).
/// @param ctx          Opaque pointer forwarded to on_ready (may be NULL).
/// @return 0 on a clean rekey-triggered exit, -1 on error.
int aivpn_run_tunnel(
    int tun_fd,
    const char *server_host,
    int server_port,
    const uint8_t *server_key,
    const uint8_t *psk,
    const uint8_t *cert_bytes,
    int cert_len,
    aivpn_ready_callback_t on_ready,
    void *ctx
);

/// Close the active UDP socket so the tunnel loop exits immediately.
/// Safe to call from any thread.
void aivpn_stop_tunnel(void);

/// Total bytes sent to the server in the current session.
int64_t aivpn_get_upload_bytes(void);

/// Total bytes received from the server in the current session.
int64_t aivpn_get_download_bytes(void);

#ifdef __cplusplus
}
#endif
