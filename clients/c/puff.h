/* puff.h
 * Copyright (C) 2002-2013 Mark Adler
 * For conditions of distribution and use, see copyright notice in puff.c
 */

#ifndef PUFF_H
#define PUFF_H

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Decompress Deflate data from source to dest.
 * On entry, *destlen is the size of dest, and *sourcelen is the size of source.
 * On return, *destlen is the actual decompressed size, and *sourcelen is the
 * number of input bytes consumed.
 * Returns 0 on success, < 0 or > 0 on error.
 *   0: success
 *   2: available inflate data did not terminate
 *  -1: output buffer overrun
 *  -2: invalid block type
 *  -3: stored block length mismatch
 *  -4: dynamic block code length error
 *  -5: invalid literal/length code
 *  -6: invalid distance code
 *  -7: missing end-of-block code
 */
int puff(unsigned char *dest,           /* pointer to destination pointer */
         unsigned long *destlen,        /* amount of output space */
         const unsigned char *source,   /* pointer to source data */
         unsigned long *sourcelen);     /* amount of input available */

#ifdef __cplusplus
}
#endif

#endif /* PUFF_H */
