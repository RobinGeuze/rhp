/*
 * ext/sockets' C side: its constants (consts.h, generated from the stub)
 * evaluated against the system's socket headers, with the macros
 * php_sockets.h, multicast.h and php's configure add (non-Windows
 * branches).
 */
/* php's configure builds with _GNU_SOURCE on Linux (SCM_CREDENTIALS). */
#ifdef __linux__
# define _GNU_SOURCE 1
#endif
/* config.m4: the RFC 3542 IPv6 API on macOS. */
#ifdef __APPLE__
# define __APPLE_USE_RFC_3542 1
#endif
#include <sys/types.h>
#include <sys/socket.h>
#include <netdb.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/un.h>
#include <arpa/inet.h>
#include <errno.h>
#include <string.h>
#include <net/if.h>
#ifdef __linux__
# include <linux/sock_diag.h>
# include <linux/filter.h>
# include <linux/if_packet.h>
# include <linux/if_ether.h>
# include <linux/udp.h>
# define LINUX 1
#endif

/* php's configure */
#define HAVE_IPV6 1
#define HAVE_SOCKETPAIR 1
#define HAVE_SHUTDOWN 1
#define HAVE_SOCKATMARK 1

/* php_sockets.h */
#ifndef MSG_EOF
# ifdef MSG_FIN
#  define MSG_EOF MSG_FIN
# endif
#endif
#ifndef MSG_WAITALL
# ifdef LINUX
#  define MSG_WAITALL 0x00000100
# else
#  define MSG_WAITALL 0x00000000
# endif
#endif
#define PHP_NORMAL_READ 0x0001
#define PHP_BINARY_READ 0x0002
#ifdef EINTR
# define PHP_SOCKET_EINTR EINTR
#endif
#ifdef EBADF
# define PHP_SOCKET_EBADF EBADF
#endif
#ifdef EACCES
# define PHP_SOCKET_EACCES EACCES
#endif
#ifdef EFAULT
# define PHP_SOCKET_EFAULT EFAULT
#endif
#ifdef EINVAL
# define PHP_SOCKET_EINVAL EINVAL
#endif
#ifdef ENFILE
# define PHP_SOCKET_ENFILE ENFILE
#endif
#ifdef EMFILE
# define PHP_SOCKET_EMFILE EMFILE
#endif
#ifdef ENAMETOOLONG
# define PHP_SOCKET_ENAMETOOLONG ENAMETOOLONG
#endif
#ifdef ENOTEMPTY
# define PHP_SOCKET_ENOTEMPTY ENOTEMPTY
#endif
#ifdef ELOOP
# define PHP_SOCKET_ELOOP ELOOP
#endif
#ifdef EWOULDBLOCK
# define PHP_SOCKET_EWOULDBLOCK EWOULDBLOCK
#endif
#ifdef EREMOTE
# define PHP_SOCKET_EREMOTE EREMOTE
#endif
#ifdef EUSERS
# define PHP_SOCKET_EUSERS EUSERS
#endif
#ifdef ENOTSOCK
# define PHP_SOCKET_ENOTSOCK ENOTSOCK
#endif
#ifdef EDESTADDRREQ
# define PHP_SOCKET_EDESTADDRREQ EDESTADDRREQ
#endif
#ifdef EMSGSIZE
# define PHP_SOCKET_EMSGSIZE EMSGSIZE
#endif
#ifdef EPROTOTYPE
# define PHP_SOCKET_EPROTOTYPE EPROTOTYPE
#endif
#ifdef ENOPROTOOPT
# define PHP_SOCKET_ENOPROTOOPT ENOPROTOOPT
#endif
#ifdef EPROTONOSUPPORT
# define PHP_SOCKET_EPROTONOSUPPORT EPROTONOSUPPORT
#endif
#ifdef ESOCKTNOSUPPORT
# define PHP_SOCKET_ESOCKTNOSUPPORT ESOCKTNOSUPPORT
#endif
#ifdef EOPNOTSUPP
# define PHP_SOCKET_EOPNOTSUPP EOPNOTSUPP
#endif
#ifdef EPFNOSUPPORT
# define PHP_SOCKET_EPFNOSUPPORT EPFNOSUPPORT
#endif
#ifdef EAFNOSUPPORT
# define PHP_SOCKET_EAFNOSUPPORT EAFNOSUPPORT
#endif
#ifdef EADDRINUSE
# define PHP_SOCKET_EADDRINUSE EADDRINUSE
#endif
#ifdef EADDRNOTAVAIL
# define PHP_SOCKET_EADDRNOTAVAIL EADDRNOTAVAIL
#endif
#ifdef ENETDOWN
# define PHP_SOCKET_ENETDOWN ENETDOWN
#endif
#ifdef ENETUNREACH
# define PHP_SOCKET_ENETUNREACH ENETUNREACH
#endif
#ifdef ENETRESET
# define PHP_SOCKET_ENETRESET ENETRESET
#endif
#ifdef ECONNABORTED
# define PHP_SOCKET_ECONNABORTED ECONNABORTED
#endif
#ifdef ECONNRESET
# define PHP_SOCKET_ECONNRESET ECONNRESET
#endif
#ifdef ENOBUFS
# define PHP_SOCKET_ENOBUFS ENOBUFS
#endif
#ifdef EISCONN
# define PHP_SOCKET_EISCONN EISCONN
#endif
#ifdef ENOTCONN
# define PHP_SOCKET_ENOTCONN ENOTCONN
#endif
#ifdef ESHUTDOWN
# define PHP_SOCKET_ESHUTDOWN ESHUTDOWN
#endif
#ifdef ETOOMANYREFS
# define PHP_SOCKET_ETOOMANYREFS ETOOMANYREFS
#endif
#ifdef ETIMEDOUT
# define PHP_SOCKET_ETIMEDOUT ETIMEDOUT
#endif
#ifdef ECONNREFUSED
# define PHP_SOCKET_ECONNREFUSED ECONNREFUSED
#endif
#ifdef EHOSTDOWN
# define PHP_SOCKET_EHOSTDOWN EHOSTDOWN
#endif
#ifdef EHOSTUNREACH
# define PHP_SOCKET_EHOSTUNREACH EHOSTUNREACH
#endif
#ifdef EALREADY
# define PHP_SOCKET_EALREADY EALREADY
#endif
#ifdef EINPROGRESS
# define PHP_SOCKET_EINPROGRESS EINPROGRESS
#endif
#ifdef EDQUOT
# define PHP_SOCKET_EDQUOT EDQUOT
#endif

/* multicast.h */
#if defined(MCAST_JOIN_GROUP) && !defined(__APPLE__) && !defined(__FreeBSD__)
# define RFC3678_API 1
# define HAS_MCAST_EXT 1
#elif defined(IP_ADD_SOURCE_MEMBERSHIP) && !defined(__APPLE__)
# define HAS_MCAST_EXT 1
#endif
#ifndef RFC3678_API
# define PHP_MCAST_JOIN_GROUP IP_ADD_MEMBERSHIP
# define PHP_MCAST_LEAVE_GROUP IP_DROP_MEMBERSHIP
# ifdef HAS_MCAST_EXT
#  define PHP_MCAST_BLOCK_SOURCE IP_BLOCK_SOURCE
#  define PHP_MCAST_UNBLOCK_SOURCE IP_UNBLOCK_SOURCE
#  define PHP_MCAST_JOIN_SOURCE_GROUP IP_ADD_SOURCE_MEMBERSHIP
#  define PHP_MCAST_LEAVE_SOURCE_GROUP IP_DROP_SOURCE_MEMBERSHIP
# endif
#else
# define PHP_MCAST_JOIN_GROUP MCAST_JOIN_GROUP
# define PHP_MCAST_LEAVE_GROUP MCAST_LEAVE_GROUP
# define PHP_MCAST_BLOCK_SOURCE MCAST_BLOCK_SOURCE
# define PHP_MCAST_UNBLOCK_SOURCE MCAST_UNBLOCK_SOURCE
# define PHP_MCAST_JOIN_SOURCE_GROUP MCAST_JOIN_SOURCE_GROUP
# define PHP_MCAST_LEAVE_SOURCE_GROUP MCAST_LEAVE_SOURCE_GROUP
#endif

typedef void (*rhp_sockets_const_cb)(void *ctx, const char *name, long long value);

/* Every constant php registers on this platform, in its order. */
void rhp_sockets_constants(rhp_sockets_const_cb cb, void *ctx)
{
#define RHP_SOCKETS_INT(name, value) cb(ctx, name, (long long)(value));
#include "consts.h"
#undef RHP_SOCKETS_INT
}

/* php_network_gethostbyname for php_set_inet_addr: 1 and the first
 * address, 2 for a non-AF_INET answer, 0 when the lookup failed (h_errno
 * says why: rhp_sockets_h_errno). */
int rhp_sockets_gethostbyname(const char *name, struct in_addr *out)
{
	struct hostent *h = gethostbyname(name);
	if (!h) {
		return 0;
	}
	if (h->h_addrtype != AF_INET) {
		return 2;
	}
	memcpy(out, h->h_addr_list[0], h->h_length);
	return 1;
}

int rhp_sockets_h_errno(void)
{
	return h_errno;
}
