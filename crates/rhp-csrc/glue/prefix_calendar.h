/*
 * Force-included (-include) into php-src's calendar C (ext/calendar's sdncal
 * conversions). Gives each global symbol it defines an rhp_ prefix (see
 * prefix_crypt.h).
 */
#ifndef RHP_PREFIX_CALENDAR_H
#define RHP_PREFIX_CALENDAR_H

#define DayOfWeek rhp_cal_DayOfWeek
#define DayNameShort rhp_cal_DayNameShort
#define DayNameLong rhp_cal_DayNameLong
#define SdnToJulian rhp_cal_SdnToJulian
#define JulianToSdn rhp_cal_JulianToSdn
#define SdnToFrench rhp_cal_SdnToFrench
#define FrenchToSdn rhp_cal_FrenchToSdn
#define FrenchMonthName rhp_cal_FrenchMonthName
#define SdnToGregorian rhp_cal_SdnToGregorian
#define GregorianToSdn rhp_cal_GregorianToSdn
#define MonthNameShort rhp_cal_MonthNameShort
#define MonthNameLong rhp_cal_MonthNameLong
#define monthsPerYear rhp_cal_monthsPerYear
#define JewishMonthNameLeap rhp_cal_JewishMonthNameLeap
#define JewishMonthName rhp_cal_JewishMonthName
#define JewishMonthHebNameLeap rhp_cal_JewishMonthHebNameLeap
#define JewishMonthHebName rhp_cal_JewishMonthHebName
#define SdnToJewish rhp_cal_SdnToJewish
#define JewishToSdn rhp_cal_JewishToSdn

#endif
