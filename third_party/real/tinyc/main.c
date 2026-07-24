/* tinyc: tiny real project — factorial + string length without libc helpers beyond printf */
int printf(char *, ...);

int fact(int n) {
    if (n <= 1)
        return 1;
    return n * fact(n - 1);
}

int slen(char *s) {
    int i;
    i = 0;
    while (s[i])
        i = i + 1;
    return i;
}

int main(void) {
    int f;
    f = fact(5);
    if (f != 120)
        return 1;
    if (slen("acc") != 3)
        return 2;
    printf("tinyc ok\n");
    return 0;
}
