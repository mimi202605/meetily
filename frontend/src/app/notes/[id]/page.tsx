import React from 'react';
import { Clock, Users, Calendar, Tag } from 'lucide-react';

interface PageProps {
  params: {
    id: string;
  };
}

interface Note {
  title: string;
  date: string;
  time?: string;
  attendees?: string[];
  tags: string[];
  content: string;
}

export function generateStaticParams() {
  // Return all possible note IDs
  return [
    { id: 'team-sync-dec-26' },
    { id: 'product-review' },
    { id: 'project-ideas' },
    { id: 'action-items' }
  ];
}

const NotePage = ({ params }: PageProps) => {
  // This would normally come from your database
  const sampleData: Record<string, Note> = {
    'team-sync-dec-26': {
      title: '团队同步 - 12月26日',
      date: '2024-12-26',
      time: '上午 10:00 - 11:00',
      attendees: ['John Doe', 'Jane Smith', 'Mike Johnson'],
      tags: ['团队同步', '每周', '产品'],
      content: `
# 会议摘要
关于 2024 年第一季度目标和当前项目状态的团队同步讨论。

## 议程事项
1. 项目状态更新
2. 2024 年第一季度规划
3. 团队问题与反馈

## 关键决策
- 优先进行第一季度的移动应用开发
- 安排每周设计评审
- 在路线图中新增两个功能

## 行动项
- [ ] John：创建项目时间表
- [ ] Jane：安排设计评审会议
- [ ] Mike：更新文档

## 备注
- 讨论了当前项目瓶颈
- 审查了上个版本的客户反馈
- 规划了即将到来的迭代的资源分配
      `
    },
    'product-review': {
      title: '产品评审',
      date: '2024-12-26',
      time: '下午 2:00 - 3:00',
      attendees: ['Sarah Wilson', 'Tom Brown', 'Alex Chen'],
      tags: ['产品', '评审', '季度'],
      content: `
# 产品评审会议

## 概述
与利益相关者的季度产品评审会议。

## 讨论要点
1. 第四季度绩效评审
2. 功能优先级排序
3. 客户反馈分析

## 行动项
- [ ] 更新产品路线图
- [ ] 安排用户调研会议
- [ ] 审查竞争对手分析
      `
    },
    'project-ideas': {
      title: '项目创意',
      date: '2024-12-26',
      tags: ['创意', '规划'],
      content: `
# 项目创意

## 新功能
1. AI 驱动的会议摘要
2. 日历集成
3. 团队协作工具

## 改进
- 增强搜索功能
- 更好的笔记组织
- 实时协作
      `
    },
    'action-items': {
      title: '行动项',
      date: '2024-12-26',
      tags: ['任务', '待办', '规划'],
      content: `
# 行动项

## 高优先级
- [ ] 将 v2.0 部署到生产环境
- [ ] 修复关键安全问题
- [ ] 完成用户文档

## 中优先级
- [ ] 更新依赖项
- [ ] 实现错误追踪
- [ ] 添加单元测试

## 低优先级
- [ ] 重构遗留代码
- [ ] 改进代码文档
- [ ] 建立开发规范
      `
    }
  };

  const note = sampleData[params.id as keyof typeof sampleData];

  if (!note) {
    return <div className="p-8">未找到笔记</div>;
  }

  return (
    <div className="p-8 max-w-4xl mx-auto">
      <div className="mb-8">
        <h1 className="text-3xl font-bold mb-4">{note.title}</h1>
        
        <div className="flex flex-wrap gap-4 text-gray-600">
          {note.date && (
            <div className="flex items-center gap-1">
              <Calendar className="w-4 h-4" />
              <span>{note.date}</span>
            </div>
          )}
          
          {note.time && (
            <div className="flex items-center gap-1">
              <Clock className="w-4 h-4" />
              <span>{note.time}</span>
            </div>
          )}
          
          {note.attendees && (
            <div className="flex items-center gap-1">
              <Users className="w-4 h-4" />
              <span>{note.attendees.join(', ')}</span>
            </div>
          )}
        </div>

        <div className="flex gap-2 mt-4">
          {note.tags.map((tag) => (
            <div key={tag} className="flex items-center gap-1 bg-blue-100 text-blue-800 px-2 py-1 rounded-full text-sm">
              <Tag className="w-3 h-3" />
              {tag}
            </div>
          ))}
        </div>
      </div>

      <div className="prose prose-blue max-w-none">
        <div dangerouslySetInnerHTML={{ __html: note.content.split('\n').map(line => {
          if (line.startsWith('# ')) {
            return `<h1>${line.slice(2)}</h1>`;
          } else if (line.startsWith('## ')) {
            return `<h2>${line.slice(3)}</h2>`;
          } else if (line.startsWith('- ')) {
            return `<li>${line.slice(2)}</li>`;
          }
          return line;
        }).join('\n') }} />
      </div>
    </div>
  );
};

export default NotePage;
