import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { SettingContainer, Textarea } from "@/components/ui";
import { ResetButton } from "../ui/ResetButton";
import { useSettings } from "../../hooks/useSettings";

// Editable system prompt for the native rephrase panel. An empty or default
// prompt is stored as null so future changes to the default still apply.
export const NativeRephrasePrompt: React.FC = React.memo(() => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const saved = getSetting("native_rephrase_prompt") ?? null;
  const [defaultPrompt, setDefaultPrompt] = useState("");
  const [draft, setDraft] = useState("");

  useEffect(() => {
    commands.getDefaultNativeRephrasePrompt().then(setDefaultPrompt);
  }, []);

  useEffect(() => {
    setDraft(saved ?? defaultPrompt);
  }, [saved, defaultPrompt]);

  const save = (value: string) => {
    const trimmed = value.trim();
    const next = trimmed === "" || trimmed === defaultPrompt ? null : trimmed;
    if (next !== saved) {
      updateSetting("native_rephrase_prompt", next);
    }
  };

  return (
    <SettingContainer
      title={t("settings.postProcessing.nativeRephrase.promptTitle")}
      description={t(
        "settings.postProcessing.nativeRephrase.promptDescription",
      )}
      descriptionMode="tooltip"
      layout="stacked"
      grouped={true}
    >
      <div className="flex gap-2 items-start">
        <Textarea
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => save(draft)}
          disabled={isUpdating("native_rephrase_prompt")}
          className="flex-1"
        />
        <ResetButton
          onClick={() => {
            setDraft(defaultPrompt);
            save(defaultPrompt);
          }}
          disabled={saved === null}
          ariaLabel={t("settings.postProcessing.nativeRephrase.resetPrompt")}
        />
      </div>
    </SettingContainer>
  );
});
NativeRephrasePrompt.displayName = "NativeRephrasePrompt";
