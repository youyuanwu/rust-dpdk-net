#ifndef DPDK_WRAPPER_H
#define DPDK_WRAPPER_H

#define _GNU_SOURCE
#include <rte_config.h>
#include <rte_eal.h>
#include <rte_ethdev.h>
#include <rte_mbuf.h>
#include <rte_lcore.h>
#include <rte_launch.h>

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

// Lcore wrapper functions (for inline functions)
unsigned rust_rte_lcore_id(void);
unsigned rust_rte_get_main_lcore(void);

// Build-config constants (expanded from #define macros for bnd-winmd)
static const unsigned int RUST_RTE_MAX_LCORE = RTE_MAX_LCORE;
static const unsigned int RUST_RTE_MAX_NUMA_NODES = RTE_MAX_NUMA_NODES;
static const uint16_t RUST_RTE_MBUF_DEFAULT_DATAROOM = RTE_MBUF_DEFAULT_DATAROOM;
static const uint16_t RUST_RTE_PKTMBUF_HEADROOM = RTE_PKTMBUF_HEADROOM;
static const uint16_t RUST_RTE_MBUF_MAX_NB_SEGS = RTE_MBUF_MAX_NB_SEGS;
static const uint32_t RUST_LCORE_ID_ANY = LCORE_ID_ANY;

// RSS hash type constants (expanded from RTE_BIT64 macros for bnd-winmd)
static const uint64_t RUST_RTE_ETH_RSS_IPV4 = RTE_ETH_RSS_IPV4;
static const uint64_t RUST_RTE_ETH_RSS_FRAG_IPV4 = RTE_ETH_RSS_FRAG_IPV4;
static const uint64_t RUST_RTE_ETH_RSS_NONFRAG_IPV4_TCP = RTE_ETH_RSS_NONFRAG_IPV4_TCP;
static const uint64_t RUST_RTE_ETH_RSS_NONFRAG_IPV4_UDP = RTE_ETH_RSS_NONFRAG_IPV4_UDP;
static const uint64_t RUST_RTE_ETH_RSS_NONFRAG_IPV4_SCTP = RTE_ETH_RSS_NONFRAG_IPV4_SCTP;
static const uint64_t RUST_RTE_ETH_RSS_NONFRAG_IPV4_OTHER = RTE_ETH_RSS_NONFRAG_IPV4_OTHER;
static const uint64_t RUST_RTE_ETH_RSS_IPV6 = RTE_ETH_RSS_IPV6;
static const uint64_t RUST_RTE_ETH_RSS_FRAG_IPV6 = RTE_ETH_RSS_FRAG_IPV6;
static const uint64_t RUST_RTE_ETH_RSS_NONFRAG_IPV6_TCP = RTE_ETH_RSS_NONFRAG_IPV6_TCP;
static const uint64_t RUST_RTE_ETH_RSS_NONFRAG_IPV6_UDP = RTE_ETH_RSS_NONFRAG_IPV6_UDP;
static const uint64_t RUST_RTE_ETH_RSS_NONFRAG_IPV6_SCTP = RTE_ETH_RSS_NONFRAG_IPV6_SCTP;
static const uint64_t RUST_RTE_ETH_RSS_NONFRAG_IPV6_OTHER = RTE_ETH_RSS_NONFRAG_IPV6_OTHER;
static const uint64_t RUST_RTE_ETH_RSS_IPV6_EX = RTE_ETH_RSS_IPV6_EX;
static const uint64_t RUST_RTE_ETH_RSS_IPV6_TCP_EX = RTE_ETH_RSS_IPV6_TCP_EX;
static const uint64_t RUST_RTE_ETH_RSS_IPV6_UDP_EX = RTE_ETH_RSS_IPV6_UDP_EX;
// Combined convenience macros
static const uint64_t RUST_RTE_ETH_RSS_IP = RTE_ETH_RSS_IP;
static const uint64_t RUST_RTE_ETH_RSS_TCP = RTE_ETH_RSS_TCP;
static const uint64_t RUST_RTE_ETH_RSS_UDP = RTE_ETH_RSS_UDP;

#endif // DPDK_WRAPPER_H
