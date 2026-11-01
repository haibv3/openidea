package com.example.openidea

import android.content.res.Configuration.UI_MODE_NIGHT_YES
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.Divider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.colorResource
import androidx.compose.ui.res.dimensionResource
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp

@Composable
fun NoteEditor(title: String, onDone: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
        horizontalAlignment = Alignment.Start,
    ) {
        Text(text = title, style = MaterialTheme.typography.titleLarge)
        Divider(color = Color(0xFFE8E0EB), thickness = 1.dp)
        Text("Sketch the layout preview toolbar")
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Checkbox(checked = true)
            Text("Done", modifier = Modifier.align(Alignment.CenterVertically))
            Spacer(Modifier.width(8.dp))
            Button(onClick = onDone) { Text("Save") }
            OutlinedButton(onClick = onDone) { Text("Cancel") }
        }
    }
}

@Preview(name = "Light", showBackground = true, widthDp = 320, heightDp = 200)
@Preview(name = "Dark", showBackground = true, backgroundColor = 0xFF1D1B20, widthDp = 320, heightDp = 200)
@Composable
private fun NoteEditorPreview() {
    NoteEditor(title = "New note", onDone = {})
}

@Preview(showBackground = true, widthDp = 240, heightDp = 240)
@Composable
private fun PalettePreview() {
    Column(
        modifier = Modifier
            .background(Color(0xFFFEF7FF), RoundedCornerShape(12.dp))
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Box(modifier = Modifier.size(48.dp).background(Color.Red))
        Box(modifier = Modifier.size(48.dp).background(Color(0xFF6750A4)))
        Spacer(Modifier.height(4.dp))
        Text("Palette", color = MaterialTheme.colorScheme.primary)
    }
}

@Preview(name = "Project resources", showBackground = true, widthDp = 280)
@Composable
private fun ResourcePreview() {
    Column(
        modifier = Modifier
            .background(colorResource(R.color.surface))
            .padding(dimensionResource(R.dimen.screen_padding)),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            stringResource(R.string.note_sample_title),
            style = MaterialTheme.typography.titleMedium,
            color = colorResource(R.color.primary),
        )
        Image(painter = painterResource(R.drawable.ic_add), contentDescription = null)
        Text(stringResource(R.string.welcome_message))
    }
}
