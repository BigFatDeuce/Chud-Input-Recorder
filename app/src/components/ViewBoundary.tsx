// Keeps a crash inside one view from taking the whole window down: the
// transport bar, emergency stop and other views stay usable.
import { AlertTriangle, RotateCcw } from "lucide-react";
import { Component, type ReactNode } from "react";
import { t } from "../i18n";

interface State {
  error: Error | null;
}

export class ViewBoundary extends Component<{ view: string; children: ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error) {
    console.error(`[view ${this.props.view}]`, error);
  }

  componentDidUpdate(prev: { view: string }) {
    // Leaving the broken view clears the error.
    if (prev.view !== this.props.view && this.state.error) this.setState({ error: null });
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="empty-hero view-crash">
        <AlertTriangle size={30} className="danger-text" />
        <h2>{t("crash.title")}</h2>
        <p className="muted">{t("crash.body")}</p>
        <pre className="mono faint">{this.state.error.message}</pre>
        <button className="btn" onClick={() => this.setState({ error: null })}>
          <RotateCcw size={14} /> {t("crash.retry")}
        </button>
      </div>
    );
  }
}
