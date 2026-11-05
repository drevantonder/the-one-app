//go:build !darwin && !linux && !windows

package main

func passkeyProbe() string { return "not probed on this platform" }
func pushProbe() string    { return "not probed on this platform" }
