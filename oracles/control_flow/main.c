int main(void) {
    int i;
    int s;
    s = 0;
    for (i = 0; i < 10; i = i + 1) {
        if (i % 2 == 0)
            continue;
        s = s + i;
    }
    /* 1+3+5+7+9 = 25 */
    return s - 25;
}
