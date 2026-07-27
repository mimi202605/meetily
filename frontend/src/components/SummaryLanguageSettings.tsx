'use client';

import { useState } from 'react';
import { Globe, Pin } from 'lucide-react';
import { Popover, PopoverTrigger, PopoverContent } from '@/components/ui/popover';
import { LanguagePickerPopover } from '@/components/LanguagePickerPopover';
import { useRecentLanguages } from '@/hooks/useRecentLanguages';
import { labelForCode } from '@/lib/summary-languages';

export function SummaryLanguageSettings() {
  const { recents, pinned, addRecent, removeRecent, setPinned } = useRecentLanguages();
  const [pickerOpen, setPickerOpen] = useState(false);

  const togglePin = (code: string) => {
    setPinned(pinned === code ? null : code);
  };

  return (
    <div className="bg-white rounded-xl border border-gray-200/70 p-6 shadow-sm hover:shadow-md transition-shadow duration-300 relative">
      <div className="flex items-start gap-3 mb-2">
        <div className="w-10 h-10 rounded-lg bg-purple-50 flex items-center justify-center flex-shrink-0">
          <Globe size={18} className="text-purple-500" />
        </div>
        <div>
          <h3 className="text-lg font-semibold text-gray-900">摘要语言</h3>
          <p className="text-sm text-gray-500 mt-0.5">
            将一种语言固定为新会议的默认语言。未固定的语言将作为摘要生成器中的快速切换选项。"自动"将使用转录内容中的主要语言。
          </p>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2 mt-4">
        {recents.map((code) => {
          const isPinned = pinned === code;
          return (
            <span
              key={code}
              className={`inline-flex items-center rounded-full border text-sm overflow-hidden transition-all duration-200 ${
                isPinned
                  ? 'bg-blue-50 border-blue-200 text-blue-800 shadow-sm'
                  : 'bg-gray-100 border-gray-200 text-gray-800 hover:bg-gray-200/60'
              }`}
            >
              <button
                type="button"
                aria-label={isPinned ? `取消固定 ${labelForCode(code)} 为默认` : `固定 ${labelForCode(code)} 为默认`}
                aria-pressed={isPinned}
                title={isPinned ? '点击取消默认' : '点击设为默认'}
                onClick={() => togglePin(code)}
                className={`flex items-center gap-1.5 pl-3 pr-2 py-1 hover:brightness-95 active:brightness-90 ${
                  isPinned ? 'text-blue-800' : 'text-gray-800'
                }`}
              >
                <Pin
                  size={14}
                  className={isPinned ? 'text-blue-600' : 'text-gray-400'}
                  fill={isPinned ? 'currentColor' : 'none'}
                />
                {labelForCode(code)}
              </button>
              <button
                type="button"
                aria-label={`移除 ${labelForCode(code)}`}
                onClick={() => removeRecent(code)}
                className={`pr-2.5 pl-0.5 py-1 leading-none ${isPinned ? 'text-blue-400 hover:text-blue-700' : 'text-gray-400 hover:text-gray-700'}`}
              >
                ×
              </button>
            </span>
          );
        })}

        <Popover open={pickerOpen} onOpenChange={setPickerOpen}>
          <PopoverTrigger asChild>
            <button
              type="button"
              disabled={recents.length >= 5}
              className="inline-flex items-center gap-1 rounded-full border border-dashed border-gray-300 px-3 py-1 text-sm text-gray-600 hover:border-blue-400 hover:text-blue-600 disabled:cursor-not-allowed disabled:opacity-50 transition-colors duration-200"
            >
              ＋ 添加语言
            </button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-auto p-0 border-0 shadow-none bg-transparent">
            <LanguagePickerPopover
              mode="settings"
              value={null}
              onChange={(code) => {
                if (code) addRecent(code);
                setPickerOpen(false);
              }}
              onClose={() => setPickerOpen(false)}
            />
          </PopoverContent>
        </Popover>
      </div>

      <p className="text-xs text-gray-400 mt-3">
        {pinned
          ? `默认：${labelForCode(pinned)} - 再次点击可取消。最多 5 个快速切换选项。`
          : '点击任意语言将其设为默认。最多 5 个快速切换选项。'}
      </p>
    </div>
  );
}
