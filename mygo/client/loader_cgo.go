//go:build cgoffi

package client

/*
#include "one_client_ffiFFI.h"
*/
import "C"

import "unsafe"

// load fills lib with the library linked into the program (an iPhone's, statically: build.sh in mygo/ios puts it in
// this package as a .syso, with uniffi's header beside it), through cgo. rustBuffer and callStatus are laid out as
// uniffi's C structs, so they pass as they are.
func load(l *lib, _ string) error {
	l.alloc = func(size uint64, st *callStatus) rustBuffer {
		return gbuf(C.ffi_one_client_ffi_rustbuffer_alloc(C.uint64_t(size), cst(st)))
	}
	l.free = func(b rustBuffer, st *callStatus) { C.ffi_one_client_ffi_rustbuffer_free(cbuf(b), cst(st)) }
	l.newDeviceSecret = func(st *callStatus) rustBuffer {
		return gbuf(C.uniffi_one_client_ffi_fn_func_new_device_secret(cst(st)))
	}
	l.deviceJoin = func(secret rustBuffer, st *callStatus) rustBuffer {
		return gbuf(C.uniffi_one_client_ffi_fn_func_device_join(cbuf(secret), cst(st)))
	}
	l.clientScreen = func(name rustBuffer, params rustBuffer, st *callStatus) rustBuffer {
		return gbuf(C.uniffi_one_client_ffi_fn_func_client_screen(cbuf(name), cbuf(params), cst(st)))
	}
	l.openNote = func(secret rustBuffer, pushed rustBuffer, st *callStatus) rustBuffer {
		return gbuf(C.uniffi_one_client_ffi_fn_func_open_note(cbuf(secret), cbuf(pushed), cst(st)))
	}
	l.deviceNew = func(secret rustBuffer, address rustBuffer, dir rustBuffer, st *callStatus) uint64 {
		return uint64(C.uniffi_one_client_ffi_fn_constructor_thisdevice_new(cbuf(secret), cbuf(address), cbuf(dir), cst(st)))
	}
	l.deviceClone = func(h uint64, st *callStatus) uint64 {
		return uint64(C.uniffi_one_client_ffi_fn_clone_thisdevice(C.uint64_t(h), cst(st)))
	}
	l.deviceFree = func(h uint64, st *callStatus) { C.uniffi_one_client_ffi_fn_free_thisdevice(C.uint64_t(h), cst(st)) }
	l.deviceKey = func(h uint64, st *callStatus) rustBuffer {
		return gbuf(C.uniffi_one_client_ffi_fn_method_thisdevice_key(C.uint64_t(h), cst(st)))
	}
	l.deviceCode = func(h uint64, st *callStatus) rustBuffer {
		return gbuf(C.uniffi_one_client_ffi_fn_method_thisdevice_code(C.uint64_t(h), cst(st)))
	}
	l.deviceOn = func(h uint64, device rustBuffer, st *callStatus) {
		C.uniffi_one_client_ffi_fn_method_thisdevice_on(C.uint64_t(h), cbuf(device), cst(st))
	}
	l.deviceAsk = func(h uint64, token rustBuffer, name rustBuffer, invite rustBuffer, st *callStatus) {
		C.uniffi_one_client_ffi_fn_method_thisdevice_ask(C.uint64_t(h), cbuf(token), cbuf(name), cbuf(invite), cst(st))
	}
	l.deviceWait = func(h uint64, token rustBuffer, st *callStatus) int8 {
		return int8(C.uniffi_one_client_ffi_fn_method_thisdevice_wait(C.uint64_t(h), cbuf(token), cst(st)))
	}
	l.deviceRemoved = func(h uint64, st *callStatus) int8 {
		return int8(C.uniffi_one_client_ffi_fn_method_thisdevice_removed(C.uint64_t(h), cst(st)))
	}
	l.deviceConnect = func(h uint64, st *callStatus) int8 {
		return int8(C.uniffi_one_client_ffi_fn_method_thisdevice_connect(C.uint64_t(h), cst(st)))
	}
	l.deviceFetch = func(h uint64, method rustBuffer, path rustBuffer, headers rustBuffer, body rustBuffer, st *callStatus) uint64 {
		return uint64(C.uniffi_one_client_ffi_fn_method_thisdevice_fetch(C.uint64_t(h), cbuf(method), cbuf(path), cbuf(headers), cbuf(body), cst(st)))
	}
	l.deviceRegisterPush = func(h uint64, via rustBuffer, address rustBuffer, env rustBuffer, topic rustBuffer, st *callStatus) {
		C.uniffi_one_client_ffi_fn_method_thisdevice_register_push(C.uint64_t(h), cbuf(via), cbuf(address), cbuf(env), cbuf(topic), cst(st))
	}
	l.deviceDeveloper = func(h uint64, st *callStatus) int8 {
		return int8(C.uniffi_one_client_ffi_fn_method_thisdevice_developer(C.uint64_t(h), cst(st)))
	}
	l.deviceSetDeveloper = func(h uint64, on int8, st *callStatus) {
		C.uniffi_one_client_ffi_fn_method_thisdevice_set_developer(C.uint64_t(h), C.int8_t(on), cst(st))
	}
	l.deviceTraces = func(h uint64, st *callStatus) rustBuffer {
		return gbuf(C.uniffi_one_client_ffi_fn_method_thisdevice_traces(C.uint64_t(h), cst(st)))
	}
	l.deviceKept = func(h uint64, app rustBuffer, op rustBuffer, input rustBuffer, st *callStatus) rustBuffer {
		return gbuf(C.uniffi_one_client_ffi_fn_method_thisdevice_kept(C.uint64_t(h), cbuf(app), cbuf(op), cbuf(input), cst(st)))
	}
	l.deviceReconnect = func(h uint64, st *callStatus) {
		C.uniffi_one_client_ffi_fn_method_thisdevice_reconnect(C.uint64_t(h), cst(st))
	}
	l.deviceSignOut = func(h uint64, st *callStatus) {
		C.uniffi_one_client_ffi_fn_method_thisdevice_sign_out(C.uint64_t(h), cst(st))
	}
	l.deviceDisconnect = func(h uint64, st *callStatus) {
		C.uniffi_one_client_ffi_fn_method_thisdevice_disconnect(C.uint64_t(h), cst(st))
	}
	l.responseClone = func(h uint64, st *callStatus) uint64 {
		return uint64(C.uniffi_one_client_ffi_fn_clone_response(C.uint64_t(h), cst(st)))
	}
	l.responseFree = func(h uint64, st *callStatus) { C.uniffi_one_client_ffi_fn_free_response(C.uint64_t(h), cst(st)) }
	l.responseStatus = func(h uint64, st *callStatus) uint16 {
		return uint16(C.uniffi_one_client_ffi_fn_method_response_status(C.uint64_t(h), cst(st)))
	}
	l.responseHeaders = func(h uint64, st *callStatus) rustBuffer {
		return gbuf(C.uniffi_one_client_ffi_fn_method_response_headers(C.uint64_t(h), cst(st)))
	}
	l.responseChunk = func(h uint64, st *callStatus) rustBuffer {
		return gbuf(C.uniffi_one_client_ffi_fn_method_response_chunk(C.uint64_t(h), cst(st)))
	}
	l.responseBytes = func(h uint64, st *callStatus) rustBuffer {
		return gbuf(C.uniffi_one_client_ffi_fn_method_response_bytes(C.uint64_t(h), cst(st)))
	}
	l.responseCancel = func(h uint64, st *callStatus) {
		C.uniffi_one_client_ffi_fn_method_response_cancel(C.uint64_t(h), cst(st))
	}
	return nil
}

func cbuf(x rustBuffer) C.RustBuffer       { return *(*C.RustBuffer)(unsafe.Pointer(&x)) }
func gbuf(x C.RustBuffer) rustBuffer       { return *(*rustBuffer)(unsafe.Pointer(&x)) }
func cst(st *callStatus) *C.RustCallStatus { return (*C.RustCallStatus)(unsafe.Pointer(st)) }
