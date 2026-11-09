/* ext/gettext's calls into libintl, through its header: Homebrew's
 * libintl.h renames them (libintl_gettext), glibc's are the libc's. */

#include <locale.h>
#include <libintl.h>

char *rhp_textdomain(const char *d) { return textdomain(d); }
char *rhp_gettext(const char *m) { return gettext(m); }
char *rhp_dgettext(const char *d, const char *m) { return dgettext(d, m); }
char *rhp_dcgettext(const char *d, const char *m, int c) { return dcgettext(d, m, c); }
char *rhp_bindtextdomain(const char *d, const char *dir) { return bindtextdomain(d, dir); }
char *rhp_ngettext(const char *m1, const char *m2, unsigned long n)
{
	return ngettext(m1, m2, n);
}
char *rhp_dngettext(const char *d, const char *m1, const char *m2, unsigned long n)
{
	return dngettext(d, m1, m2, n);
}
char *rhp_dcngettext(const char *d, const char *m1, const char *m2, unsigned long n, int c)
{
	return dcngettext(d, m1, m2, n, c);
}
char *rhp_bind_textdomain_codeset(const char *d, const char *cs)
{
	return bind_textdomain_codeset(d, cs);
}
