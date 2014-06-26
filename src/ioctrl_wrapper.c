#include <stdlib.h>
#include <errno.h>
#include <sys/termios.h>
#include <sys/ioctl.h>
#include <sys/fcntl.h>


// create the phantom type interface for termios

// allocate a new termios
struct termios *new_termios() {
    struct termios *t;
    t = malloc(sizeof(struct termios));
    return t;
}

// deallocate a termios
void free_termios(struct termios *to_free) {
    free(to_free);
}

// implement a copy function for termios
struct termios *clone_termios(struct termios *to_clone) {
    // allocate a new one
    struct termios *the_copy = new_termios();

    // a shallow copy of a termios should be deep as it only contains values
    *the_copy = *to_clone;

    return the_copy;
}

// wrapper on open file
int open_port_file(char *path) {
    return open(path, O_WRONLY | O_NOCTTY | O_NONBLOCK );
}


// wrappers on ioctrl functions

// set port access exclusive to me
int ioctrl_tiocexcl(int fd) {
    return ioctl(fd, TIOCEXCL);
}

// set the port options
int tcsetattr_tcsanow(int fd, struct termios *options) {
    return tcsetattr(fd, TCSANOW, options);
}

// empty everything in the port
int tcflush_io(int fd) {
    return tcflush(fd, TCIOFLUSH);
}

// set options on this termios object
void set_options_enttec(struct termios *options) {

    (*options).c_cflag = (CS8 | CSTOPB | CLOCAL | CREAD);
    (*options).c_lflag &= ~(ICANON | ECHO | ECHOE | ISIG);
    (*options).c_oflag &= ~OPOST;
    (*options).c_cc[ VMIN ] = 1;
    (*options).c_cc[ VTIME ] = 0;

}

// probably not necessary
// set RS485 for sending
int ioctrl_tiocmgetandset(int fd) {
    int flag;
    int ret = ioctl(fd, TIOCMGET, &flag);
    if (ret != 0) {
        return ret;
    }

    flag &= ~TIOCM_RTS;
    ret = ioctl(fd, TIOCMSET, &flag);
    return ret;
}




