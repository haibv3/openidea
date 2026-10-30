/**
 * Live source for plugin previews: an editor contribution on files a preview
 * provider covers publishes the document text (debounced while typing), per
 * path — the same mechanism as the Markdown preview's live source.
 */
import { ViewPlugin, type EditorView, type ViewUpdate } from "@codemirror/view";
import { create } from "zustand";
import { registerEditorContribution } from "@/editor/api";
import { useProject } from "@/platform/project";
import { usePlugins } from "../store";
import { findPreviewProvider } from "./match";

export const usePluginPreviewSource = create<Record<string, { text: string } | undefined>>(() => ({}));

const TEXT_DELAY = 120;

function publish(path: string, text: string) {
  usePluginPreviewSource.setState({ [path]: { text } });
}

function livePlugin(path: string) {
  return ViewPlugin.fromClass(
    class {
      private timer: ReturnType<typeof setTimeout> | undefined;

      constructor(private view: EditorView) {
        publish(path, view.state.doc.toString());
      }

      update(u: ViewUpdate) {
        if (!u.docChanged) return;
        clearTimeout(this.timer);
        this.timer = setTimeout(() => publish(path, this.view.state.doc.toString()), TEXT_DELAY);
      }

      destroy() {
        clearTimeout(this.timer);
        // Flush the latest edit for a preview that outlives this view.
        publish(path, this.view.state.doc.toString());
      }
    },
  );
}

export function registerPluginPreviewLiveSource(): () => void {
  return registerEditorContribution({
    id: "plugins.previewLive",
    extensions: (ctx) =>
      !ctx.virtual &&
      findPreviewProvider(ctx.path, usePlugins.getState().plugins, useProject.getState().project?.root ?? null) !== null
        ? [livePlugin(ctx.path)]
        : [],
  });
}
