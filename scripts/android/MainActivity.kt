package com.auralis.v2

import android.os.Bundle
import android.webkit.WebView

class MainActivity : TauriActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        WebView.setWebContentsDebuggingEnabled(true)
    }
}
