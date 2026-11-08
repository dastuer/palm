package io.palm.client

import com.journeyapps.barcodescanner.CaptureActivity

/** 扫码页强制竖屏（库默认跟随传感器，手机平放会横过来，与主界面竖屏不一致） */
class PortraitScanActivity : CaptureActivity()
