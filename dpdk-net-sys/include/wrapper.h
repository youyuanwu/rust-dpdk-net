#ifndef DPDK_WRAPPER_H
#define DPDK_WRAPPER_H

#define _GNU_SOURCE
#include <rte_config.h>
#include <rte_eal.h>
#include <rte_ethdev.h>
#include <rte_mbuf.h>

// Wrapper functions for accessing rte_errno (per-lcore macro)
int rust_get_rte_errno(void);
void rust_set_rte_errno(int err);

// Mbuf wrapper functions (for inline functions that bindgen can't handle)
struct rte_mbuf* rust_pktmbuf_alloc(struct rte_mempool *mp);
void rust_pktmbuf_free(struct rte_mbuf *m);
char* rust_pktmbuf_mtod(struct rte_mbuf *m);
uint16_t rust_pktmbuf_data_len(const struct rte_mbuf *m);
void rust_pktmbuf_set_data_len(struct rte_mbuf *m, uint16_t len);
uint32_t rust_pktmbuf_pkt_len(const struct rte_mbuf *m);
void rust_pktmbuf_set_pkt_len(struct rte_mbuf *m, uint32_t len);
uint16_t rust_pktmbuf_headroom(const struct rte_mbuf *m);
uint16_t rust_pktmbuf_tailroom(const struct rte_mbuf *m);
char* rust_pktmbuf_append(struct rte_mbuf *m, uint16_t len);
char* rust_pktmbuf_prepend(struct rte_mbuf *m, uint16_t len);
char* rust_pktmbuf_adj(struct rte_mbuf *m, uint16_t len);
int rust_pktmbuf_trim(struct rte_mbuf *m, uint16_t len);
void rust_pktmbuf_reset(struct rte_mbuf *m);
uint16_t rust_pktmbuf_data_room_size(struct rte_mempool *mp);

// Ethernet RX/TX burst wrappers (static inline functions)
uint16_t rust_eth_rx_burst(uint16_t port_id, uint16_t queue_id,
                           struct rte_mbuf **rx_pkts, uint16_t nb_pkts);
uint16_t rust_eth_tx_burst(uint16_t port_id, uint16_t queue_id,
                           struct rte_mbuf **tx_pkts, uint16_t nb_pkts);

#endif // DPDK_WRAPPER_H
