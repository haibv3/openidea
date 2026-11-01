package com.example.openidea.ui

import android.content.Context
import android.util.AttributeSet
import com.google.android.material.button.MaterialButton

/** Library-free custom button: the preview must render it as a Material button. */
class BrandButton @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : MaterialButton(context, attrs)
