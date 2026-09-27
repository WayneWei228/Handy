import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface NativeRephraseToggleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const NativeRephraseToggle: React.FC<NativeRephraseToggleProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const enabled = getSetting("native_rephrase_enabled") ?? true;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(nextEnabled) =>
          updateSetting("native_rephrase_enabled", nextEnabled)
        }
        isUpdating={isUpdating("native_rephrase_enabled")}
        label={t("settings.postProcessing.nativeRephrase.title")}
        description={t("settings.postProcessing.nativeRephrase.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  });
