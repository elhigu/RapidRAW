import { Loader2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useElapsedSeconds } from '../../hooks/useElapsedSeconds';
import { selectMigraphxCompileStart, useProcessStore } from '../../store/useProcessStore';
import { TextColors, TextVariants } from '../../types/typography';
import Text from './Text';

export default function MigraphxCompileStatus({ compact = false }: { compact?: boolean }) {
  const { t } = useTranslation();
  const startedAt = useProcessStore(selectMigraphxCompileStart);
  const seconds = useElapsedSeconds(startedAt);
  if (seconds === null) return null;
  return (
    <div
      className={`${compact ? 'h-9' : 'h-12'} px-4 rounded-full bg-surface flex items-center gap-2 shrink-0`}
      data-tooltip={t('library.jobs.migraphxCompiling')}
    >
      <Loader2 size={16} className="animate-spin shrink-0 text-accent" />
      <Text as="span" variant={TextVariants.small} color={TextColors.primary} className="whitespace-nowrap">
        {t('library.jobs.kinds.migraphxCompile')}
      </Text>
      <Text as="span" variant={TextVariants.small} color={TextColors.secondary} className="tabular-nums">
        {t('library.jobs.elapsed', { seconds })}
      </Text>
    </div>
  );
}
