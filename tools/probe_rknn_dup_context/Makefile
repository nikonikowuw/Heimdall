CC ?= gcc
CFLAGS ?= -O2 -Wall -Wextra -pthread
LDFLAGS ?= -ldl -lpthread

TARGET = probe_rknn_dup_context
SRCS = main.c

.PHONY: all clean

all: $(TARGET)

$(TARGET): $(SRCS)
	$(CC) $(CFLAGS) $(SRCS) $(LDFLAGS) -o $(TARGET)

clean:
	rm -f $(TARGET)
