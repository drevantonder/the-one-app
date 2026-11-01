package main

import "runtime/debug"

// version is the commit this driver was built from, for the App Store's
// Drivers to show.
func version() string {
	if info, ok := debug.ReadBuildInfo(); ok {
		for _, s := range info.Settings {
			if s.Key == "vcs.revision" {
				return s.Value
			}
		}
	}
	return ""
}
