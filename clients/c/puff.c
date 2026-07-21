/* puff.c
 * Copyright (C) 2002-2013 Mark Adler
 * For conditions of distribution and use, see copyright notice below.
 *
 * This software is provided 'as-is', without any express or implied
 * warranty.  In no event will the author be held liable for any damages
 * arising from the use of this software.
 *
 * Permission is granted to anyone to use this software for any purpose,
 * including commercial applications, and to alter it and redistribute it
 * freely, subject to the following restrictions:
 *
 * 1. The origin of this software must not be misrepresented; you must not
 *    claim that you wrote the original software. If you use this software
 *    in a product, an acknowledgment in the product documentation would be
 *    appreciated but is not required.
 * 2. Altered source versions must be plainly marked as such, and must not be
 *    misrepresented as being the original software.
 * 3. This notice may not be removed or altered from any source distribution.
 *
 * Mark Adler
 * madler@alumni.caltech.edu
 */

#include <setjmp.h>             /* for setjmp(), longjmp(), and jmp_buf */
#include "puff.h"

#define MAXBITS 15              /* maximum bits in a code */
#define MAXLCODES 286           /* maximum number of literal/length codes */
#define MAXDCODES 30            /* maximum number of distance codes */
#define MAXCODES (MAXLCODES+MAXDCODES)  /* maximum codes defines in a block */
#define FIXLCODES 288           /* number of fixed literal/length codes */

/* input and output state */
struct state {
    /* output state */
    unsigned char *out;         /* output buffer */
    unsigned long outlen;       /* available space at out */
    unsigned long outcnt;       /* bytes written to out so far */

    /* input state */
    const unsigned char *in;    /* input buffer */
    unsigned long inlen;        /* available input bytes */
    unsigned long incnt;        /* input bytes read so far */
    int bitbuf;                 /* bit buffer */
    int bitcnt;                 /* number of bits in bit buffer */

    /* input limit error return state for longjmp */
    jmp_buf env;
};

/*
 * Return need bits from the input stream.  This always leaves bitbuf
 * consisting of the low bitcnt bits.
 */
static int bits(struct state *s, int need)
{
    long val;

    /* load at least need bits into val */
    val = s->bitbuf;
    while (s->bitcnt < need) {
        if (s->incnt >= s->inlen)
            longjmp(s->env, 1);         /* out of input */
        val |= (long)(s->in[s->incnt++]) << s->bitcnt;
        s->bitcnt += 8;
    }

    /* drop need bits and store remaining bits in bitbuf */
    s->bitbuf = (int)(val >> need);
    s->bitcnt -= need;

    /* return need bits, masking the required number of bits */
    return (int)(val & ((1L << need) - 1));
}

/*
 * Process a stored block.
 */
static int stored(struct state *s)
{
    unsigned len;       /* length of stored block */

    /* discard leftover bits from current byte (up to 7 bits) */
    s->bitbuf = 0;
    s->bitcnt = 0;

    /* get length and check complement */
    if (s->incnt + 4 > s->inlen)
        return 2;                               /* not enough input */
    len = s->in[s->incnt];
    len |= s->in[s->incnt + 1] << 8;
    if (s->in[s->incnt + 2] != ((~len) & 0xff) ||
        s->in[s->incnt + 3] != ((~len >> 8) & 0xff))
        return -3;                              /* error in len shut */
    s->incnt += 4;

    /* copy len bytes from in to out */
    if (s->incnt + len > s->inlen)
        return 2;                               /* not enough input */
    if (s->outcnt + len > s->outlen)
        return -1;                              /* not enough output space */
    while (len--)
        s->out[s->outcnt++] = s->in[s->incnt++];
    return 0;
}

/*
 * Huffman code decoding table structure.
 */
struct huffman {
    short *count;       /* number of codes of each length */
    short *symbol;      /* symbols in order of increasing code length */
};

/*
 * Decode a code from the stream s using huffman table h.
 */
static int decode(struct state *s, const struct huffman *h)
{
    int len;            /* current number of bits in code */
    int code;           /* len bits pulled from input stream */
    int first;          /* first code of length len */
    int count;          /* number of codes of length len */
    int index;          /* index of first code of length len in symbol table */

    code = 0;
    first = 0;
    index = 0;
    for (len = 1; len <= MAXBITS; len++) {
        code |= bits(s, 1);
        count = h->count[len];
        if (code - count < first)
            return h->symbol[index + (code - first)];
        index += count;
        first += count;
        first <<= 1;
        code <<= 1;
    }
    return -10;                         /* ran out of codes */
}

/*
 * Given the list of code lengths length[0..n-1], construct the Huffman table h.
 */
static int construct(struct huffman *h, const short *length, int n)
{
    int symbol;         /* element of length[] being processed */
    int len;            /* length of symbol currently being processed */
    int left;           /* number of prefixes left to allocate */
    short offs[MAXBITS + 1];    /* offsets in symbol table for each length */

    /* count number of codes of each length */
    for (len = 0; len <= MAXBITS; len++)
        h->count[len] = 0;
    for (symbol = 0; symbol < n; symbol++)
        (h->count[length[symbol]])++;   /* assumes length[] all <= MAXBITS */
    if (h->count[0] == n)               /* complete, but empty code set */
        return 0;                       /* consider it valid */

    /* check for an over-subscribed or incomplete set of lengths */
    left = 1;
    for (len = 1; len <= MAXBITS; len++) {
        left <<= 1;
        left -= h->count[len];
        if (left < 0)
            return left;                /* over-subscribed */
    }

    /* generate offsets into symbol table for each length for sorting */
    offs[1] = 0;
    for (len = 1; len < MAXBITS; len++)
        offs[len + 1] = offs[len] + h->count[len];

    /* put symbols in table sorted by length, by symbol order within each length */
    for (symbol = 0; symbol < n; symbol++)
        if (length[symbol] != 0)
            h->symbol[offs[length[symbol]]++] = symbol;

    /* return zero for complete set, positive for incomplete set */
    return left;
}

/*
 * Decode data until end-of-block code using lencode and distcode.
 */
static int codes(struct state *s,
                 const struct huffman *lencode,
                 const struct huffman *distcode)
{
    int symbol;         /* decoded symbol */
    int len;            /* length for copy */
    unsigned dist;      /* distance for copy */
    static const short lens[29] = { /* Size base for length codes 257..285 */
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31,
        35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258};
    static const short lext[29] = { /* Extra bits for length codes 257..285 */
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2,
        3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0};
    static const short dists[30] = { /* Offset base for distance codes 0..29 */
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193,
        257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
        8193, 12289, 16385, 24577};
    static const short dext[30] = { /* Extra bits for distance codes 0..29 */
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6,
        7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13};

    /* decode codes until end-of-block (256) */
    do {
        symbol = decode(s, lencode);
        if (symbol < 0)
            return symbol;              /* invalid symbol */
        if (symbol < 256) {             /* literal: copy byte to output */
            if (s->outcnt >= s->outlen)
                return -1;
            s->out[s->outcnt++] = symbol;
        }
        else if (symbol > 256) {        /* length code */
            /* get length */
            symbol -= 257;
            if (symbol >= 29)
                return -5;              /* invalid fixed code */
            len = lens[symbol] + bits(s, lext[symbol]);

            /* get distance */
            symbol = decode(s, distcode);
            if (symbol < 0)
                return symbol;          /* invalid symbol */
            dist = dists[symbol] + bits(s, dext[symbol]);

            /* copy length bytes from distance bytes back */
            if (dist > s->outcnt)
                return -6;              /* distance too far back */
            if (s->outcnt + len > s->outlen)
                return -1;              /* not enough output space */
            while (len--) {
                s->out[s->outcnt] = s->out[s->outcnt - dist];
                s->outcnt++;
            }
        }
    } while (symbol != 256);            /* end of block symbol */

    /* done with a valid block */
    return 0;
}

/*
 * Process a fixed Huffman block.
 */
static int fixed(struct state *s)
{
    static int built = 0;
    static short lencnt[MAXBITS + 1], lensym[FIXLCODES];
    static short distcnt[MAXBITS + 1], distsym[MAXDCODES];
    static struct huffman lencode = {lencnt, lensym};
    static struct huffman distcode = {distcnt, distsym};

    /* build fixed huffman tables if not already built */
    if (!built) {
        int symbol;
        short lengths[FIXLCODES];

        /* literal/length table */
        for (symbol = 0; symbol < 144; symbol++)
            lengths[symbol] = 8;
        for (; symbol < 256; symbol++)
            lengths[symbol] = 9;
        for (; symbol < 280; symbol++)
            lengths[symbol] = 7;
        for (; symbol < FIXLCODES; symbol++)
            lengths[symbol] = 8;
        construct(&lencode, lengths, FIXLCODES);

        /* distance table */
        for (symbol = 0; symbol < MAXDCODES; symbol++)
            lengths[symbol] = 5;
        construct(&distcode, lengths, MAXDCODES);

        built = 1;
    }

    /* decode data until end-of-block code */
    return codes(s, &lencode, &distcode);
}

/*
 * Process a dynamic Huffman block.
 */
static int dynamic(struct state *s)
{
    int nlen, ndist, ncode;             /* number of lengths in descriptor */
    int index;                          /* index of code length being retrieved */
    int err;                            /* error code from construct or codes */
    short lengths[MAXCODES];            /* descriptor code lengths */
    short lencnt[MAXBITS + 1], lensym[MAXLCODES];    /* lencode memory */
    short distcnt[MAXBITS + 1], distsym[MAXDCODES]; /* distcode memory */
    struct huffman lencode, distcode;   /* length and distance codes */
    static const short order[19] =      /* permutation of code lengths */
        {16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15};

    /* set up code tables */
    lencode.count = lencnt;
    lencode.symbol = lensym;
    distcode.count = distcnt;
    distcode.symbol = distsym;

    /* get number of lengths in each table, check limit */
    nlen = bits(s, 5) + 257;
    ndist = bits(s, 5) + 1;
    ncode = bits(s, 4) + 4;
    if (nlen > MAXLCODES || ndist > MAXDCODES)
        return -4;                      /* bad counts */

    /* read code length code lengths (pre-code lengths) */
    for (index = 0; index < ncode; index++)
        lengths[order[index]] = bits(s, 3);
    for (; index < 19; index++)
        lengths[order[index]] = 0;

    /* build pre-code huffman table */
    err = construct(&lencode, lengths, 19);
    if (err != 0)
        return -4;                      /* bad pre-code lengths */

    /* read code lengths for literal/length and distance tables */
    index = 0;
    while (index < nlen + ndist) {
        int symbol;
        int len;

        symbol = decode(s, &lencode);
        if (symbol < 0)
            return symbol;              /* invalid symbol */
        if (symbol < 16)                /* length in 0..15 */
            lengths[index++] = symbol;
        else {                          /* repeat length or zero */
            len = 0;
            if (symbol == 16) {
                if (index == 0)
                    return -4;          /* no length to repeat */
                len = lengths[index - 1];
                symbol = 3 + bits(s, 2);
            }
            else if (symbol == 17)
                symbol = 3 + bits(s, 3);
            else
                symbol = 11 + bits(s, 7);
            if (index + symbol > nlen + ndist)
                return -4;              /* repeat more than total lengths */
            while (symbol--)
                lengths[index++] = len;
        }
    }

    /* check for end-of-block code -- if not, return error */
    if (lengths[256] == 0)
        return -9;

    /* build literal/length table */
    err = construct(&lencode, lengths, nlen);
    if (err < 0 || (err > 0 && nlen - lencode.count[0] != 1))
        return -4;                      /* incomplete code set */

    /* build distance table */
    err = construct(&distcode, lengths + nlen, ndist);
    if (err < 0 || (err > 0 && ndist - distcode.count[0] != 1))
        return -4;                      /* incomplete code set */

    /* decode data until end-of-block code */
    return codes(s, &lencode, &distcode);
}

/*
 * Inflate source to dest.
 */
int puff(unsigned char *dest,
         unsigned long *destlen,
         const unsigned char *source,
         unsigned long *sourcelen)
{
    int last, type;             /* block information */
    int err;                    /* error code */
    struct state s;             /* input/output state */

    /* initialize state */
    s.out = dest;
    s.outlen = *destlen;
    s.outcnt = 0;
    s.in = source;
    s.inlen = *sourcelen;
    s.incnt = 0;
    s.bitbuf = 0;
    s.bitcnt = 0;

    if (setjmp(s.env) != 0)
        err = 2;                /* ran out of input */
    else {
        /* process blocks until last block or error */
        do {
            last = bits(&s, 1);
            type = bits(&s, 2);
            err = type == 0 ? stored(&s) :
                 (type == 1 ? fixed(&s) :
                 (type == 2 ? dynamic(&s) : -2));
        } while (err == 0 && !last);
    }

    /* update lengths */
    *destlen = s.outcnt;
    *sourcelen = s.incnt;
    return err;
}
