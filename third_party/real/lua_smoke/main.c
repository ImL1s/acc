/* lua_smoke: tiny stack machine (not real Lua) */
int printf(char *, ...);

int add2(int a, int b) {
    return a + b;
}

int run(int x, int y) {
    int t;
    t = add2(x, y);
    return t;
}

int main(void) {
    int r;
    r = run(10, 32);
    if (r != 42)
        return 1;
    printf("lua_smoke ok\n");
    return 0;
}
