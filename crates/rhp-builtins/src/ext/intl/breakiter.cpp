/* The ICU C++ behind IntlBreakIterator (breakiterator.rs): php-src's
 * PHP::CodePointBreakIterator (ext/intl/breakiterator/
 * codepointiterator_internal.cpp, PHP License 3.01, Gustavo Lopes), kept
 * in namespace PHP so its typeid name matches php's, and C entry points
 * for the BreakIterator calls php's methods make. */

#include <cstdlib>
#include <cstring>
#include <memory>
#include <new>
#include <typeinfo>

#include <unicode/brkiter.h>
#include <unicode/rbbi.h>
#include <unicode/uchriter.h>
#include <unicode/unistr.h>
#include <unicode/utext.h>

using icu::BreakIterator;
using icu::CharacterIterator;
using icu::Locale;
using icu::RuleBasedBreakIterator;
using icu::UCharCharacterIterator;
using icu::UnicodeString;

namespace PHP {

class CodePointBreakIterator : public BreakIterator {
public:
	static UClassID getStaticClassID();
	CodePointBreakIterator();
	CodePointBreakIterator(const CodePointBreakIterator &other);
	CodePointBreakIterator &operator=(const CodePointBreakIterator &that);
	~CodePointBreakIterator() override;
	bool operator==(const BreakIterator &that) const override;
	CodePointBreakIterator *clone(void) const override;
	UClassID getDynamicClassID(void) const override;
	CharacterIterator &getText(void) const override;
	UText *getUText(UText *fillIn, UErrorCode &status) const override;
	void setText(const UnicodeString &text) override;
	void setText(UText *text, UErrorCode &status) override;
	void adoptText(CharacterIterator *it) override;
	int32_t first(void) override;
	int32_t last(void) override;
	int32_t previous(void) override;
	int32_t next(void) override;
	int32_t current(void) const override;
	int32_t following(int32_t offset) override;
	int32_t preceding(int32_t offset) override;
	UBool isBoundary(int32_t offset) override;
	int32_t next(int32_t n) override;
	CodePointBreakIterator *createBufferClone(void *stackBuffer, int32_t &BufferSize, UErrorCode &status) override;
	CodePointBreakIterator &refreshInputText(UText *input, UErrorCode &status) override;
	inline UChar32 getLastCodePoint() { return this->lastCodePoint; }

private:
	UText *fText;
	UChar32 lastCodePoint;
	mutable CharacterIterator *fCharIter;
	inline void clearCurrentCharIter()
	{
		delete this->fCharIter;
		this->fCharIter = NULL;
		this->lastCodePoint = U_SENTINEL;
	}
};

} // namespace PHP

using namespace PHP;

UOBJECT_DEFINE_RTTI_IMPLEMENTATION(CodePointBreakIterator)

CodePointBreakIterator::CodePointBreakIterator() : BreakIterator(), lastCodePoint(U_SENTINEL), fCharIter(NULL)
{
	UErrorCode uec = UErrorCode();
	this->fText = utext_openUChars(NULL, NULL, 0, &uec);
}

CodePointBreakIterator::CodePointBreakIterator(const PHP::CodePointBreakIterator &other)
	: BreakIterator(other), fText(NULL), lastCodePoint(U_SENTINEL), fCharIter(NULL)
{
	*this = other;
}

CodePointBreakIterator &CodePointBreakIterator::operator=(const CodePointBreakIterator &that)
{
	UErrorCode uec = UErrorCode();
	if (this == &that) {
		return *this;
	}
	this->fText = utext_clone(this->fText, that.fText, false, true, &uec);
	clearCurrentCharIter();
	this->lastCodePoint = that.lastCodePoint;
	return *this;
}

CodePointBreakIterator::~CodePointBreakIterator()
{
	if (this->fText) {
		utext_close(this->fText);
	}
	clearCurrentCharIter();
}

bool CodePointBreakIterator::operator==(const BreakIterator &that) const
{
	if (typeid(*this) != typeid(that)) {
		return false;
	}
	const CodePointBreakIterator &that2 = static_cast<const CodePointBreakIterator &>(that);
	return utext_equals(this->fText, that2.fText);
}

CodePointBreakIterator *CodePointBreakIterator::clone(void) const { return new CodePointBreakIterator(*this); }

CharacterIterator &CodePointBreakIterator::getText(void) const
{
	if (this->fCharIter == NULL) {
		static const UChar c = 0;
		this->fCharIter = new UCharCharacterIterator(&c, 0);
	}
	return *this->fCharIter;
}

UText *CodePointBreakIterator::getUText(UText *fillIn, UErrorCode &status) const
{
	return utext_clone(fillIn, this->fText, false, true, &status);
}

void CodePointBreakIterator::setText(const UnicodeString &text)
{
	UErrorCode uec = UErrorCode();
	this->fText = utext_openConstUnicodeString(this->fText, &text, &uec);
	clearCurrentCharIter();
}

void CodePointBreakIterator::setText(UText *text, UErrorCode &status)
{
	if (U_FAILURE(status)) {
		return;
	}
	this->fText = utext_clone(this->fText, text, false, true, &status);
	clearCurrentCharIter();
}

void CodePointBreakIterator::adoptText(CharacterIterator *it)
{
	UErrorCode uec = UErrorCode();
	clearCurrentCharIter();
	this->fCharIter = it;
	this->fText = utext_openCharacterIterator(this->fText, it, &uec);
}

int32_t CodePointBreakIterator::first(void)
{
	UTEXT_SETNATIVEINDEX(this->fText, 0);
	this->lastCodePoint = U_SENTINEL;
	return 0;
}

int32_t CodePointBreakIterator::last(void)
{
	int32_t pos = (int32_t) utext_nativeLength(this->fText);
	UTEXT_SETNATIVEINDEX(this->fText, pos);
	this->lastCodePoint = U_SENTINEL;
	return pos;
}

int32_t CodePointBreakIterator::previous(void)
{
	this->lastCodePoint = UTEXT_PREVIOUS32(this->fText);
	if (this->lastCodePoint == U_SENTINEL) {
		return BreakIterator::DONE;
	}
	return (int32_t) UTEXT_GETNATIVEINDEX(this->fText);
}

int32_t CodePointBreakIterator::next(void)
{
	this->lastCodePoint = UTEXT_NEXT32(this->fText);
	if (this->lastCodePoint == U_SENTINEL) {
		return BreakIterator::DONE;
	}
	return (int32_t) UTEXT_GETNATIVEINDEX(this->fText);
}

int32_t CodePointBreakIterator::current(void) const { return (int32_t) UTEXT_GETNATIVEINDEX(this->fText); }

int32_t CodePointBreakIterator::following(int32_t offset)
{
	this->lastCodePoint = utext_next32From(this->fText, offset);
	if (this->lastCodePoint == U_SENTINEL) {
		return BreakIterator::DONE;
	}
	return (int32_t) UTEXT_GETNATIVEINDEX(this->fText);
}

int32_t CodePointBreakIterator::preceding(int32_t offset)
{
	this->lastCodePoint = utext_previous32From(this->fText, offset);
	if (this->lastCodePoint == U_SENTINEL) {
		return BreakIterator::DONE;
	}
	return (int32_t) UTEXT_GETNATIVEINDEX(this->fText);
}

UBool CodePointBreakIterator::isBoundary(int32_t offset)
{
	utext_setNativeIndex(this->fText, offset);
	return (offset == utext_getNativeIndex(this->fText));
}

int32_t CodePointBreakIterator::next(int32_t n)
{
	UBool res = utext_moveIndex32(this->fText, n);
	if (res) {
		this->lastCodePoint = utext_current32(this->fText);
		return (int32_t) UTEXT_GETNATIVEINDEX(this->fText);
	}
	this->lastCodePoint = U_SENTINEL;
	return BreakIterator::DONE;
}

CodePointBreakIterator *CodePointBreakIterator::createBufferClone(void *, int32_t &bufferSize, UErrorCode &status)
{
	if (U_FAILURE(status)) {
		return NULL;
	}
	if (bufferSize <= 0) {
		bufferSize = sizeof(CodePointBreakIterator) + 16;
		return NULL;
	}
	CodePointBreakIterator *clonedBI = new CodePointBreakIterator(*this);
	status = U_SAFECLONE_ALLOCATED_WARNING;
	return clonedBI;
}

CodePointBreakIterator &CodePointBreakIterator::refreshInputText(UText *input, UErrorCode &status)
{
	if (U_FAILURE(status)) {
		return *this;
	}
	if (input == NULL) {
		status = U_ILLEGAL_ARGUMENT_ERROR;
		return *this;
	}
	int64_t pos = utext_getNativeIndex(this->fText);
	this->fText = utext_clone(this->fText, input, false, true, &status);
	if (U_FAILURE(status)) {
		return *this;
	}
	utext_setNativeIndex(this->fText, pos);
	if (utext_getNativeIndex(fText) != pos) {
		status = U_ILLEGAL_ARGUMENT_ERROR;
	}
	return *this;
}

static UChar *brk_copy_ustr(const UnicodeString &s, int32_t *len)
{
	*len = s.length();
	UChar *out = (UChar *) malloc(sizeof(UChar) * (*len + 1));
	if (out) {
		memcpy(out, s.getBuffer(), sizeof(UChar) * *len);
	}
	return out;
}

extern "C" {

/* `_breakiter_factory`: 0 word, 1 line, 2 character, 3 sentence, 4 title. */
void *rhp_brk_create(int32_t kind, const char *locale, UErrorCode *status)
{
	Locale l = Locale::createFromName(locale);
	switch (kind) {
		case 0: return BreakIterator::createWordInstance(l, *status);
		case 1: return BreakIterator::createLineInstance(l, *status);
		case 2: return BreakIterator::createCharacterInstance(l, *status);
		case 3: return BreakIterator::createSentenceInstance(l, *status);
		default: return BreakIterator::createTitleInstance(l, *status);
	}
}
void *rhp_brk_codepoint_new(void) { return new CodePointBreakIterator(); }
/* `breakiterator_object_create`'s class: 1 rule based, 2 code point, 0 other. */
int32_t rhp_brk_kind(const void *b)
{
	UClassID id = ((const BreakIterator *) b)->getDynamicClassID();
	if (id == RuleBasedBreakIterator::getStaticClassID()) {
		return 1;
	}
	if (id == CodePointBreakIterator::getStaticClassID()) {
		return 2;
	}
	return 0;
}
const char *rhp_brk_type_name(const void *b) { return typeid(*(const BreakIterator *) b).name(); }
int32_t rhp_brk_equal(const void *a, const void *b) { return *(const BreakIterator *) a == *(const BreakIterator *) b; }
void *rhp_brk_clone(const void *b) { return ((const BreakIterator *) b)->clone(); }
void rhp_brk_delete(void *b) { delete (BreakIterator *) b; }
/* `setText`: the UTF-8 bytes stay alive with the object (php keeps the zval). */
void rhp_brk_set_text(void *b, const char *s, int64_t len, UErrorCode *status)
{
	UText *ut = utext_openUTF8(NULL, s, len, status);
	if (U_FAILURE(*status)) {
		return;
	}
	((BreakIterator *) b)->setText(ut, *status);
	utext_close(ut);
}
/* 0 first, 1 last, 2 previous, 3 next, 4 current, 5 following(n),
 * 6 preceding(n), 7 next(n), 8 isBoundary(n). */
int32_t rhp_brk_op(void *p, int32_t op, int32_t n)
{
	BreakIterator *b = (BreakIterator *) p;
	switch (op) {
		case 0: return b->first();
		case 1: return b->last();
		case 2: return b->previous();
		case 3: return b->next();
		case 4: return b->current();
		case 5: return b->following(n);
		case 6: return b->preceding(n);
		case 7: return b->next(n);
		default: return b->isBoundary(n);
	}
}
/* `getLocale`: the name, into `buf` (NUL-terminated, truncated to cap). */
void rhp_brk_locale(const void *b, int32_t type, char *buf, int32_t cap, UErrorCode *status)
{
	Locale l = ((const BreakIterator *) b)->getLocale((ULocDataLocaleType) type, *status);
	strncpy(buf, l.getName(), cap - 1);
	buf[cap - 1] = 0;
}
/* IntlRuleBasedBreakIterator::__construct: from rules (UTF-16) or compiled
 * rules; the parse error into `pe`. */
void *rhp_rbbi_new(const UChar *rules, int32_t len, UParseError *pe, UErrorCode *status)
{
	UnicodeString s(rules, len);
	RuleBasedBreakIterator *r = new RuleBasedBreakIterator(s, *pe, *status);
	if (U_FAILURE(*status)) {
		delete r;
		return NULL;
	}
	return r;
}
void *rhp_rbbi_new_compiled(const uint8_t *rules, uint32_t len, UErrorCode *status)
{
	RuleBasedBreakIterator *r = new RuleBasedBreakIterator(rules, len, *status);
	if (U_FAILURE(*status)) {
		delete r;
		return NULL;
	}
	return r;
}
UChar *rhp_rbbi_rules(const void *b, int32_t *len)
{
	return brk_copy_ustr(((const RuleBasedBreakIterator *) b)->getRules(), len);
}
int32_t rhp_rbbi_rule_status(const void *b) { return ((const RuleBasedBreakIterator *) b)->getRuleStatus(); }
int32_t rhp_rbbi_rule_status_vec(void *b, int32_t *out, int32_t cap, UErrorCode *status)
{
	return ((RuleBasedBreakIterator *) b)->getRuleStatusVec(out, cap, *status);
}
const uint8_t *rhp_rbbi_binary_rules(void *b, uint32_t *len)
{
	return ((RuleBasedBreakIterator *) b)->getBinaryRules(*len);
}
int32_t rhp_cpbi_last_code_point(void *b) { return ((CodePointBreakIterator *) b)->getLastCodePoint(); }

} /* extern "C" */
