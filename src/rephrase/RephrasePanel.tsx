import { listen } from "@tauri-apps/api/event";
import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import "./RephrasePanel.css";

type Row = "original" | "native";

// Mirrors `RephraseView` in src-tauri/src/rephrase.rs.
interface RephraseView {
  original: string;
  native: string | null;
  loading: boolean;
  error: string | null;
  already_natural: boolean;
  selected: Row;
}

// Purely presentational: the backend owns selection and key handling, since
// this window never takes focus.
const RephrasePanel: React.FC = () => {
  const { t } = useTranslation();
  const [view, setView] = useState<RephraseView | null>(null);
  const [position, setPosition] = useState<"top" | "bottom">("bottom");

  useEffect(() => {
    const unlisteners = [
      listen<RephraseView>("rephrase-state", (event) => setView(event.payload)),
      listen<"top" | "bottom">("rephrase-position", (event) =>
        setPosition(event.payload),
      ),
    ];
    return () => {
      unlisteners.forEach((p) => p.then((unlisten) => unlisten()));
    };
  }, []);

  if (!view) return null;

  const showNative = view.loading || (view.native && !view.already_natural);

  return (
    <div className={`rp-root rp-${position}`}>
      <div className="rp-card">
        <div
          className={`rp-row ${view.selected === "original" ? "rp-selected" : ""}`}
        >
          <span className="rp-label">{t("rephrasePanel.original")}</span>
          <span className="rp-text">{view.original}</span>
        </div>

        {showNative && (
          <div
            className={`rp-row ${view.selected === "native" ? "rp-selected" : ""}`}
          >
            <span className="rp-label">{t("rephrasePanel.native")}</span>
            <span className={`rp-text ${view.loading ? "rp-loading" : ""}`}>
              {view.loading ? t("rephrasePanel.loading") : view.native}
            </span>
          </div>
        )}

        {view.already_natural && (
          <div className="rp-note">{t("rephrasePanel.alreadyNatural")}</div>
        )}
        {view.error && (
          <div className="rp-note rp-error">
            {t("rephrasePanel.error", { error: view.error })}
          </div>
        )}

        <div className="rp-hint">{t("rephrasePanel.hint")}</div>
      </div>
    </div>
  );
};

export default RephrasePanel;
