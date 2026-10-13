package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	strict := false
	args := os.Args[1:]
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--strict":
			strict = true
		}
	}
	if v := os.Getenv("APP_STRICT"); v != "" {
		if b, err := strconv.ParseBool(v); err == nil {
			strict = b
		}
	}
	if strict {
		fmt.Println("strict mode on")
	} else {
		fmt.Println("strict mode off")
	}
}
