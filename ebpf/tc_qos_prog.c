// SPDX-License-Identifier: GPL-2.0
// AIVPN TC egress QoS program — per-client token bucket rate limiting + DSCP marking.
//
// Attached to the TUN egress path (tc qdisc add ... bpf obj tc_qos_prog.o sec tc).
// BPF map `qos_rules` (LRU_HASH): key = client VPN IP (u32 BE), value = QosRule.
// Graceful degradation: if the map is absent the kernel refuses to load — server
// falls back to userspace-only enforcement (no crash).

#include <linux/bpf.h>
#include <linux/pkt_cls.h>
#include <linux/ip.h>
#include <linux/if_ether.h>
#include <bpf/bpf_helpers.h>
#include <bpf/bpf_endian.h>

#define NS_PER_SEC  1000000000ULL
#define MAX_CLIENTS 512

struct qos_rule {
    __u64 rate_bps;         // bytes per second limit (0 = unlimited)
    __u64 tokens;           // current token bucket fill (bytes)
    __u64 last_refill_ns;   // last refill timestamp (bpf_ktime_get_ns)
    __u8  dscp;             // DSCP value to mark (0 = no mark)
    __u8  pad[7];
};

struct {
    __uint(type, BPF_MAP_TYPE_LRU_HASH);
    __uint(max_entries, MAX_CLIENTS);
    __type(key, __u32);          // client VPN IP (network byte order)
    __type(value, struct qos_rule);
} qos_rules SEC(".maps");

SEC("tc")
int tc_qos_egress(struct __sk_buff *skb) {
    void *data     = (void *)(long)skb->data;
    void *data_end = (void *)(long)skb->data_end;

    struct ethhdr *eth = data;
    if ((void *)(eth + 1) > data_end)
        return TC_ACT_OK;
    if (eth->h_proto != bpf_htons(ETH_P_IP))
        return TC_ACT_OK;

    struct iphdr *iph = (void *)(eth + 1);
    if ((void *)(iph + 1) > data_end)
        return TC_ACT_OK;

    // Egress: destination IP = client VPN IP
    __u32 dst_ip = iph->daddr;
    struct qos_rule *rule = bpf_map_lookup_elem(&qos_rules, &dst_ip);
    if (!rule)
        return TC_ACT_OK;

    __u64 now = bpf_ktime_get_ns();

    // DSCP marking
    if (rule->dscp) {
        __u8 new_tos = (iph->tos & 0x03) | (rule->dscp << 2);
        if (iph->tos != new_tos) {
            bpf_skb_store_bytes(skb,
                sizeof(struct ethhdr) + offsetof(struct iphdr, tos),
                &new_tos, 1, BPF_F_RECOMPUTE_CSUM);
        }
    }

    // Token bucket
    if (rule->rate_bps == 0)
        return TC_ACT_OK;

    __u64 burst     = rule->rate_bps / 10; // 100 ms burst
    __u64 elapsed   = now - rule->last_refill_ns;
    // Cap elapsed time to 1 second to prevent overflow in rate_bps * elapsed
    if (elapsed > NS_PER_SEC) {
        elapsed = NS_PER_SEC;
    }
    __u64 new_tokens = rule->tokens + (rule->rate_bps * elapsed / NS_PER_SEC);
    if (new_tokens > burst)
        new_tokens = burst;

    __u32 pkt_len = skb->len;
    if (new_tokens < pkt_len) {
        rule->tokens = new_tokens;
        rule->last_refill_ns = now;
        return TC_ACT_SHOT;
    }

    rule->tokens = new_tokens - pkt_len;
    rule->last_refill_ns = now;
    return TC_ACT_OK;
}

char _license[] SEC("license") = "GPL";
