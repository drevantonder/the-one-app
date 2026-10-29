//go:build !linux && !darwin

package devices

import "errors"

func disk() (Space, error) { return Space{}, errors.ErrUnsupported }
