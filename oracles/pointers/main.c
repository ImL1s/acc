int main(void) {
    int x;
    int *p;
    int **pp;
    x = 0;
    p = &x;
    pp = &p;
    **pp = 7;
    return *p - 7;
}
