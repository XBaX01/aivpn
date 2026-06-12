// SPDX-License-Identifier: GPL-2.0
/*
 * xdp_prog.c — AIVPN XDP early-filter program.
 *
 * Compiled with: clang -O2 -target bpf -c xdp_prog.c -o xdp_prog.o
 * Attached with: ip link set dev <iface> xdp obj xdp_prog.o sec xdp
 *
 * Purpose:
 *   First-stage DDoS protection at NIC RX level, before socket buffer
 *   allocation.  Validates that UDP packets on the configured VPN port carry
 *   a plausible AIVPN resonance-tag header (correct size, non-expired
 *   timestamp).  Clearly invalid packets are dropped in XDP before any
 *   kernel networking work is done.
 *
 *   Legitimate packets are forwarded with XDP_PASS to the normal network
 *   stack, where the kernel module's sk_data_ready hook performs full
 *   tag validation and decryption.
 *
 * Configuration (BPF ARRAY map "xdp_config", pinned at
 *   /sys/fs/bpf/aivpn/xdp_config):
 *   key 0 → u64  VPN UDP destination port (host byte order; 0 = pass all UDP)
 *   key 1 → u64  tag acceptance window in milliseconds (default 10 000 ms)
 *
 * Protocol layout at UDP payload offset 0:
 *   [8 bytes]  resonance tag  (first 8 bytes = LE u64 millisecond timestamp)
 *   [1 byte]   pad_len
 *   [1 byte]   inner_type / control sub-type
 *   [...]      encrypted payload + Poly1305 tag
 *
 * Minimum valid payload: tag(8) + pad_len(1) + inner_type(1) +
 *                        Poly1305(16) = 26 bytes.
 */

#include <linux/bpf.h>
#include <linux/if_ether.h>
#include <linux/in.h>
#include <linux/ip.h>
#include <linux/udp.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>

/* Minimum AIVPN UDP payload: tag + pad_len + inner_type + Poly1305 auth tag */
#define AIVPN_MIN_PAYLOAD 26

/* BPF ARRAY map: 2 slots (index 0 = port, index 1 = window_ms) */
struct {
	__uint(type,        BPF_MAP_TYPE_ARRAY);
	__uint(max_entries, 2);
	__type(key,         __u32);
	__type(value,       __u64);
} xdp_config SEC(".maps");

SEC("xdp")
int aivpn_xdp_filter(struct xdp_md *ctx)
{
	void *data     = (void *)(long)ctx->data;
	void *data_end = (void *)(long)ctx->data_end;

	/* ── Ethernet ─────────────────────────────────────────────────── */
	struct ethhdr *eth = data;
	if ((void *)(eth + 1) > data_end)
		return XDP_DROP;
	if (eth->h_proto != bpf_htons(ETH_P_IP))
		return XDP_PASS; /* non-IPv4: ignore */

	/* ── IPv4 ─────────────────────────────────────────────────────── */
	struct iphdr *ip = (void *)(eth + 1);
	if ((void *)(ip + 1) > data_end)
		return XDP_PASS;
	if (ip->protocol != IPPROTO_UDP)
		return XDP_PASS;

	__u32 ip_hlen = (__u32)ip->ihl * 4;
	if (ip_hlen < sizeof(*ip))
		return XDP_DROP;

	/* ── UDP ──────────────────────────────────────────────────────── */
	struct udphdr *udp = (void *)ip + ip_hlen;
	if ((void *)(udp + 1) > data_end)
		return XDP_PASS;

	/* ── Port check ───────────────────────────────────────────────── */
	__u32  key0    = 0;
	__u64 *pcfg    = bpf_map_lookup_elem(&xdp_config, &key0);
	__u16  vpn_port = pcfg ? (__u16)*pcfg : 0;
	if (vpn_port != 0 && udp->dest != bpf_htons(vpn_port))
		return XDP_PASS; /* not our port: leave untouched */

	/* ── Minimum size check ───────────────────────────────────────── */
	__u16 udp_total = bpf_ntohs(udp->len); /* includes UDP header */
	if (udp_total < sizeof(*udp) + AIVPN_MIN_PAYLOAD)
		return XDP_DROP; /* too short to be a valid AIVPN packet */

	/* ── Resonance-tag timestamp check ───────────────────────────── */
	__u8 *payload = (void *)(udp + 1);
	if ((void *)(payload + 8) > data_end)
		return XDP_DROP;

	__u64 tag_ts = 0;
	__builtin_memcpy(&tag_ts, payload, 8); /* LE u64 milliseconds */

	__u64 now_ms = bpf_ktime_get_ns() / 1000000ULL;

	__u32  key1   = 1;
	__u64 *wcfg   = bpf_map_lookup_elem(&xdp_config, &key1);
	__u64  win_ms = (wcfg && *wcfg) ? *wcfg : 10000ULL;

	__u64 delta = now_ms > tag_ts ? now_ms - tag_ts : tag_ts - now_ms;
	if (delta > win_ms)
		return XDP_DROP; /* expired or future-dated tag: drop */

	return XDP_PASS;
}

char _license[] SEC("license") = "GPL";
