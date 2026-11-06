/*
 * Force-included (-include) into libavifinfo's avifinfo.c. Gives each global
 * symbol it defines an rhp_ prefix (see prefix_crypt.h).
 */
#ifndef RHP_PREFIX_AVIFINFO_H
#define RHP_PREFIX_AVIFINFO_H

#define AvifInfoIdentify rhp_AvifInfoIdentify
#define AvifInfoGetFeatures rhp_AvifInfoGetFeatures
#define AvifInfoIdentifyStream rhp_AvifInfoIdentifyStream
#define AvifInfoGetFeaturesStream rhp_AvifInfoGetFeaturesStream

#endif
