package app.theone.lynxspike;

import android.app.Application;
import com.facebook.drawee.backends.pipeline.Fresco;
import com.lynx.service.image.LynxImageService;
import com.lynx.service.log.LynxLogService;
import com.lynx.tasm.LynxEnv;
import com.lynx.tasm.service.LynxServiceCenter;

public class App extends Application {
    @Override public void onCreate() {
        super.onCreate();
        Fresco.initialize(this);
        LynxServiceCenter.inst().registerService(LynxImageService.getInstance());
        LynxServiceCenter.inst().registerService(LynxLogService.INSTANCE);
        LynxEnv.inst().init(this, null, null, null);
    }
}
