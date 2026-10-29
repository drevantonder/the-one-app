//go:build linux || darwin

package devices

import (
	"os"
	"runtime"

	"golang.org/x/sys/unix"
)

// disk measures the volume user files live on: on macOS, the Data volume,
// since / is the read-only system.
func disk() (Space, error) {
	path := "/"
	if runtime.GOOS == "darwin" {
		if _, err := os.Stat("/System/Volumes/Data"); err == nil {
			path = "/System/Volumes/Data"
		}
	}
	var st unix.Statfs_t
	if err := unix.Statfs(path, &st); err != nil {
		return Space{}, err
	}
	bs := uint64(st.Bsize)
	used := (st.Blocks - st.Bfree) * bs
	return Space{Used: used, Total: used + st.Bavail*bs}, nil
}
