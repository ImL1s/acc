/* shim for acc userspace builds — termios/stat macros as integer constants */
#ifndef ACC_TERMIOS_SHIM_H
#define ACC_TERMIOS_SHIM_H
#ifndef BRKINT
#define BRKINT 0000002
#endif
#ifndef ICRNL
#define ICRNL 0000400
#endif
#ifndef INPCK
#define INPCK 0000020
#endif
#ifndef ISTRIP
#define ISTRIP 0000040
#endif
#ifndef IXON
#define IXON 0002000
#endif
#ifndef OPOST
#define OPOST 0000001
#endif
#ifndef CS8
#define CS8 0000060
#endif
#ifndef ECHO
#define ECHO 0000010
#endif
#ifndef ICANON
#define ICANON 0000002
#endif
#ifndef IEXTEN
#define IEXTEN 0000400
#endif
#ifndef ISIG
#define ISIG 0000001
#endif
#ifndef VMIN
#define VMIN 6
#endif
#ifndef VTIME
#define VTIME 5
#endif
#ifndef TCSANOW
#define TCSANOW 0
#endif
#ifndef S_IRWXG
#define S_IRWXG 00070
#endif
#ifndef S_IRWXO
#define S_IRWXO 00007
#endif
#endif
