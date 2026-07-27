#include <stdio.h>
#include <assert.h>

/* Enum definitions with constant expressions, bit shifts, and casts */
enum Tokens {
    TOK_ID = 257,
    TOK_INT = 258,
    TOK_CREATE = 265,
    TOK_BOOTSTRAP = (TOK_CREATE + 11), /* 276 */
    TOK_ROWTYPE_OID = (1 << 8) | 22,    /* 278 */
    TOK_UNEVALUATED = (int)(TOK_ROWTYPE_OID + 2)
};

#define MACRO_CASE_1 (TOK_ID + 1)
#define MACRO_CASE_2 (TOK_INT + 10)

/* Flex/Bison state and table simulation */
static const signed char yycheck[] = { -1, 0, 1, 2, 25, 26, 27, 28, -1, 10 };
static const short yybase[] = { -1, 0, 5, 10 };

static int simulate_flex_scanner(int act) {
    int token = 0;
    switch (act) {
        case 0:
            token = 0;
            break;
        case TOK_ID:
            token = 100;
            break;
        case TOK_CREATE:
            token = 200;
            break;
        case TOK_BOOTSTRAP:
            token = 300;
            break;
        case TOK_ROWTYPE_OID:
            token = 400;
            break;
        case MACRO_CASE_1:
            token = 500;
            break;
        case MACRO_CASE_2:
            token = 600;
            break;
        case (TOK_ROWTYPE_OID + 1):
            token = 700;
            break;
        default:
            token = -1;
            break;
    }
    return token;
}

static int simulate_bison_table_lookup(int state, int tok) {
    /* 32-bit signed index calculation with signed char array lookup */
    int yyn = yybase[state];
    int idx = yyn + tok;
    signed char check_val = yycheck[idx];
    if (check_val == -1) {
        return 999;
    }
    return check_val;
}

int main(void) {
    /* 1. Test Flex scanner switch dispatch with enums/macros/casts */
    assert(simulate_flex_scanner(0) == 0);
    assert(simulate_flex_scanner(TOK_ID) == 100);
    assert(simulate_flex_scanner(TOK_CREATE) == 200);
    assert(simulate_flex_scanner(TOK_BOOTSTRAP) == 300);
    assert(simulate_flex_scanner(TOK_ROWTYPE_OID) == 400);
    assert(simulate_flex_scanner(MACRO_CASE_1) == 500);
    assert(simulate_flex_scanner(MACRO_CASE_2) == 600);
    assert(simulate_flex_scanner(TOK_ROWTYPE_OID + 1) == 700);
    assert(simulate_flex_scanner(9999) == -1);

    /* 2. Test signed char table lookup (-1 sign extension) */
    assert(simulate_bison_table_lookup(0, 1) == 999);
    assert(simulate_bison_table_lookup(1, 1) == 0);
    assert(simulate_bison_table_lookup(1, 2) == 1);

    printf("PG_SWITCH_FLEX_BISON_OK\n");
    return 0;
}
