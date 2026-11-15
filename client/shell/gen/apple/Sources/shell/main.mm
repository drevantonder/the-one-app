#include "bindings/bindings.h"
#import <Foundation/Foundation.h>

int main(int argc, char * argv[]) {
	// The app keeps its client in the folder it shares with its notification extension, its App Group, which
	// reads this device's key there to open what APNs brings (Notifications/NotificationService.swift).
	NSString *group = [[NSBundle mainBundle] objectForInfoDictionaryKey:@"AppGroup"];
	NSURL *shared = group ? [[NSFileManager defaultManager] containerURLForSecurityApplicationGroupIdentifier:group] : nil;
	if (shared) setenv("ONE_HOME", shared.path.fileSystemRepresentation, 1);
	ffi::start_app();
	return 0;
}
