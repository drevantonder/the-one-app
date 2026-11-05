package main

import (
	"fmt"
	"syscall"
	"unsafe"
)

// passkeyProbe asks webauthn.dll (Windows Hello's passkeys, which WebView2 lacks for a non-https origin) its API version
// and whether a platform authenticator is there: WebAuthNAuthenticatorGetAssertion takes the same HWND MyGo gives.
func passkeyProbe() string {
	dll, err := syscall.LoadDLL("webauthn.dll")
	if err != nil {
		return "webauthn.dll: " + err.Error()
	}
	version, _, _ := dll.MustFindProc("WebAuthNGetApiVersionNumber").Call()
	var available int32
	hr, _, _ := dll.MustFindProc("WebAuthNIsUserVerifyingPlatformAuthenticatorAvailable").Call(uintptr(unsafe.Pointer(&available)))
	return fmt.Sprintf("webauthn.dll API v%d, platform authenticator available: %v (HRESULT 0x%x)", version, available != 0, uint32(hr))
}

// pushProbe says whether the app has a package identity, which a WNS channel (PushNotificationManager, or
// PushNotificationChannelManager's for an application) needs: a plain .exe has none, an MSIX-packaged one does.
func pushProbe() string {
	k := syscall.MustLoadDLL("kernel32.dll").MustFindProc("GetCurrentPackageFullName")
	var n uint32 = 256
	buf := make([]uint16, n)
	r, _, _ := k.Call(uintptr(unsafe.Pointer(&n)), uintptr(unsafe.Pointer(&buf[0])))
	if r == 15700 { // APPMODEL_ERROR_NO_PACKAGE
		return "no package identity: a WNS channel needs the app packaged (MSIX, as windows/ is); a plain MyGo .exe can't register"
	}
	return "packaged as " + syscall.UTF16ToString(buf[:n]) + ": a WNS channel can be asked for"
}
