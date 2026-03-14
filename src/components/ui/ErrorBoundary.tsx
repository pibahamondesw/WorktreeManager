import { Component, ReactNode } from "react";

interface Props {
  children: ReactNode;
  fallbackClassName?: string;
}

interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("ErrorBoundary caught:", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div
          className={`flex items-center justify-center p-6 ${this.props.fallbackClassName ?? ""}`}
        >
          <div className="text-center max-w-sm space-y-3">
            <div className="w-10 h-10 rounded-lg bg-danger/10 flex items-center justify-center mx-auto">
              <svg
                width="20"
                height="20"
                viewBox="0 0 16 16"
                fill="currentColor"
                className="text-danger"
              >
                <path d="M8 1a7 7 0 1 0 0 14A7 7 0 0 0 8 1zm-.5 3a.5.5 0 0 1 1 0v4a.5.5 0 0 1-1 0V4zm.5 7.5a.75.75 0 1 1 0-1.5.75.75 0 0 1 0 1.5z" />
              </svg>
            </div>
            <p className="text-sm font-medium text-text-primary">Something went wrong</p>
            <p className="text-xs text-text-muted select-text cursor-text break-words">
              {this.state.error.message}
            </p>
            <button
              onClick={() => this.setState({ error: null })}
              className="px-4 py-1.5 text-xs font-medium rounded-lg bg-bg-tertiary text-text-primary hover:bg-bg-hover border border-border transition-colors cursor-pointer"
            >
              Try Again
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}
