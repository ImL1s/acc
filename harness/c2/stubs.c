#include <stdio.h>
#include <stddef.h>
int __ggcc_errno;
int ll2string(char *d,size_t n,long long v){return snprintf(d,n,"%lld",v);}
int ull2string(char *d,size_t n,unsigned long long v){return snprintf(d,n,"%llu",v);}
unsigned int current_task(void){return 0;}
int task_for_pid(unsigned int a,int b,unsigned int *c){(void)a;(void)b;(void)c;return 1;}
int task_info(unsigned int a,int b,int *c,int *d){(void)a;(void)b;(void)c;(void)d;return 1;}
int proc_pidinfo(int a,int b,unsigned long long c,void *d,int e){(void)a;(void)b;(void)c;(void)d;(void)e;return 0;}
/* SQLite zipfile.c helpers sometimes not emitted under ggcc; weak stubs for testfixture link. */
int zipfileInflate(void *a, void *b, void *c, void *d){(void)a;(void)b;(void)c;(void)d;return 0;}
int zipfileDeflate(void *a, void *b, void *c, void *d){(void)a;(void)b;(void)c;(void)d;return 0;}
