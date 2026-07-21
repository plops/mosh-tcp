CC ?= gcc
CFLAGS ?= -O3 -Wall -Wextra -std=c99 -pedantic
LDFLAGS ?=

TARGET = mosh-tcp-client-c
SRCS = mosh_tcp_client.c puff.c
OBJS = $(SRCS:.c=.o)

all: $(TARGET)

$(TARGET): $(OBJS)
	$(CC) $(CFLAGS) -o $@ $(OBJS) $(LDFLAGS)

%.o: %.c
	$(CC) $(CFLAGS) -c $< -o $@

sanitizer: CFLAGS += -fsanitize=address,undefined -g -O1
sanitizer: clean $(TARGET)

clean:
	rm -f $(OBJS) $(TARGET)

.PHONY: all clean sanitizer
