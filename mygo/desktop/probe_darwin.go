package main

import (
	"fmt"
	"os"
	"time"
	"unsafe"

	"github.com/ebitengine/purego"
	"github.com/ebitengine/purego/objc"
	"github.com/egoist/mygo"
)

// The Mac's native gaps, through purego's Objective-C runtime: a passkey (ASAuthorizationController) and push
// registration (NSApplication's registerForRemoteNotifications). Both need the app signed with entitlements a plain
// MyGo build hasn't (associated domains; aps-environment): each probe says what macOS answered.

func nsString(s string) objc.ID {
	return objc.ID(objc.GetClass("NSString")).Send(objc.RegisterName("stringWithUTF8String:"), s+"\x00")
}

func describe(err objc.ID) string {
	if err == 0 {
		return "no error"
	}
	desc := err.Send(objc.RegisterName("localizedDescription"))
	code := objc.Send[int](err, objc.RegisterName("code"))
	domain := objc.ID(err.Send(objc.RegisterName("domain")))
	return fmt.Sprintf("%s %d: %s", goString(domain), code, goString(desc))
}

func goString(s objc.ID) string {
	if s == 0 {
		return ""
	}
	p := objc.Send[uintptr](s, objc.RegisterName("UTF8String"))
	if p == 0 {
		return ""
	}
	var b []byte
	for i := uintptr(0); ; i++ {
		c := *(*byte)(unsafe.Pointer(p + i))
		if c == 0 {
			break
		}
		b = append(b, c)
	}
	return string(b)
}

var passkeyDone = make(chan string, 1)

// passkeyProbe asks macOS for a passkey assertion for ONE_PASSKEY_RP (an install's host): without the
// associated-domains entitlement, macOS refuses before showing anything.
func passkeyProbe() string {
	rp := os.Getenv("ONE_PASSKEY_RP")
	if rp == "" {
		return "skipped (set ONE_PASSKEY_RP to an install's host)"
	}
	if _, err := purego.Dlopen("/System/Library/Frameworks/AuthenticationServices.framework/AuthenticationServices", purego.RTLD_NOW|purego.RTLD_GLOBAL); err != nil {
		return "AuthenticationServices: " + err.Error()
	}
	delegate, err := objc.RegisterClass("OnePasskeyDelegate", objc.GetClass("NSObject"),
		[]*objc.Protocol{objc.GetProtocol("ASAuthorizationControllerDelegate"), objc.GetProtocol("ASAuthorizationControllerPresentationContextProviding")}, nil,
		[]objc.MethodDef{
			{Cmd: objc.RegisterName("authorizationController:didCompleteWithAuthorization:"), Fn: func(self objc.ID, _ objc.SEL, _ objc.ID, auth objc.ID) {
				passkeyDone <- "got a credential: " + goString(objc.ID(objc.ID(auth.Send(objc.RegisterName("credential"))).Send(objc.RegisterName("description"))))
			}},
			{Cmd: objc.RegisterName("authorizationController:didCompleteWithError:"), Fn: func(self objc.ID, _ objc.SEL, _ objc.ID, e objc.ID) {
				passkeyDone <- "macOS refused: " + describe(e)
			}},
			{Cmd: objc.RegisterName("presentationAnchorForAuthorizationController:"), Fn: func(self objc.ID, _ objc.SEL, _ objc.ID) objc.ID {
				if homeWin == nil {
					return 0
				}
				return objc.ID(homeWin.NativeHandle())
			}},
		})
	if err != nil {
		return "delegate: " + err.Error()
	}
	mygo.RunOnMain(func() {
		provider := objc.ID(objc.GetClass("ASAuthorizationPlatformPublicKeyCredentialProvider")).Send(objc.RegisterName("alloc")).
			Send(objc.RegisterName("initWithRelyingPartyIdentifier:"), nsString(rp))
		challenge := objc.ID(objc.GetClass("NSData")).Send(objc.RegisterName("dataWithBytes:length:"), unsafe.Pointer(&[32]byte{1, 2, 3}), 32)
		request := provider.Send(objc.RegisterName("createCredentialAssertionRequestWithChallenge:"), challenge)
		requests := objc.ID(objc.GetClass("NSArray")).Send(objc.RegisterName("arrayWithObject:"), request)
		controller := objc.ID(objc.GetClass("ASAuthorizationController")).Send(objc.RegisterName("alloc")).Send(objc.RegisterName("initWithAuthorizationRequests:"), requests)
		d := objc.ID(delegate).Send(objc.RegisterName("new"))
		controller.Send(objc.RegisterName("setDelegate:"), d)
		controller.Send(objc.RegisterName("setPresentationContextProvider:"), d)
		controller.Send(objc.RegisterName("performRequests"))
	})
	select {
	case r := <-passkeyDone:
		return r
	case <-time.After(60 * time.Second):
		return "no answer in 60 s (a sheet may be waiting for the person)"
	}
}

var pushDone = make(chan string, 1)

// pushProbe registers for remote notifications, adding the two callbacks to MyGo's application delegate's class.
func pushProbe() string {
	lib, err := purego.Dlopen("/usr/lib/libobjc.A.dylib", purego.RTLD_NOW|purego.RTLD_GLOBAL)
	if err != nil {
		return err.Error()
	}
	var addMethod func(cls objc.Class, sel objc.SEL, imp objc.IMP, types string) bool
	purego.RegisterLibFunc(&addMethod, lib, "class_addMethod")
	var objectGetClass func(objc.ID) objc.Class
	purego.RegisterLibFunc(&objectGetClass, lib, "object_getClass")
	time.Sleep(2 * time.Second) // the app's delegate is set once it's running
	result := "no answer in 30 s"
	mygo.RunOnMain(func() {
		app := objc.ID(objc.GetClass("NSApplication")).Send(objc.RegisterName("sharedApplication"))
		delegate := app.Send(objc.RegisterName("delegate"))
		if delegate == 0 {
			result = "no application delegate to answer"
			return
		}
		cls := objectGetClass(delegate)
		addMethod(cls, objc.RegisterName("application:didRegisterForRemoteNotificationsWithDeviceToken:"), objc.NewIMP(func(self objc.ID, _ objc.SEL, _ objc.ID, token objc.ID) {
			pushDone <- fmt.Sprintf("registered: an APNs token of %d bytes", objc.Send[int](token, objc.RegisterName("length")))
		}), "v@:@@")
		addMethod(cls, objc.RegisterName("application:didFailToRegisterForRemoteNotificationsWithError:"), objc.NewIMP(func(self objc.ID, _ objc.SEL, _ objc.ID, e objc.ID) {
			pushDone <- "macOS refused: " + describe(e)
		}), "v@:@@")
		app.Send(objc.RegisterName("registerForRemoteNotifications"))
	})
	select {
	case r := <-pushDone:
		return r
	case <-time.After(30 * time.Second):
		return result
	}
}
