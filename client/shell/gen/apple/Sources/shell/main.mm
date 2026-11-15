#include "bindings/bindings.h"
#import <Foundation/Foundation.h>

int main(int argc, char * argv[]) {
	// The app keeps its client in the folder it shares with its notification extension, its App Group, which reads
	// this device's key there to open what APNs brings (Notifications/NotificationService.swift). iOS gives that folder
	// only to an app signed with the group; without it, the app keeps its client in its own (`client::phone_home`).
	NSString *group = [[NSBundle mainBundle] objectForInfoDictionaryKey:@"AppGroup"];
	NSURL *shared = group ? [[NSFileManager defaultManager] containerURLForSecurityApplicationGroupIdentifier:group] : nil;
	if (shared) setenv("ONE_HOME", shared.path.fileSystemRepresentation, 1);
	ffi::start_app();
	return 0;
}
