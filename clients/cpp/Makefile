CXX ?= g++
CC ?= gcc
CXXFLAGS ?= -O3 -Wall -Wextra -std=c++20 -pedantic
CFLAGS ?= -O3 -Wall -Wextra -std=c99 -pedantic
LDFLAGS ?=

TARGET = mosh-tcp-client-cpp
OBJS = mosh_tcp_client.o puff.o

all: $(TARGET)

$(TARGET): $(OBJS)
	$(CXX) $(CXXFLAGS) -o $@ $(OBJS) $(LDFLAGS)

mosh_tcp_client.o: mosh_tcp_client.cpp
	$(CXX) $(CXXFLAGS) -c $< -o $@

puff.o: ../c/puff.c
	$(CC) $(CFLAGS) -c $< -o $@

sanitizer: CXXFLAGS += -fsanitize=address,undefined -g -O1
sanitizer: CFLAGS += -fsanitize=address,undefined -g -O1
sanitizer: LDFLAGS += -fsanitize=address,undefined
sanitizer: clean $(TARGET)

clean:
	rm -f $(OBJS) $(TARGET)

.PHONY: all clean sanitizer
