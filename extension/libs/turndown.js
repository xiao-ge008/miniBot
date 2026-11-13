/**
 * miniBot 极简 HTML -> Markdown 转换器
 */
(function(global) {
  class MiniTurndown {
    turndown(htmlOrNode) {
      let div;
      if (typeof htmlOrNode === 'string') {
        div = document.createElement('div');
        div.innerHTML = htmlOrNode;
      } else {
        div = htmlOrNode.cloneNode(true);
      }

      return this.processNode(div).replace(/\n{3,}/g, '\n\n').trim();
    }

    processNode(node) {
      if (!node) return '';
      if (node.nodeType === Node.TEXT_NODE) {
        return node.textContent.replace(/[\t\r\n]+/g, ' ');
      }
      if (node.nodeType !== Node.ELEMENT_NODE) {
        return '';
      }

      const tag = node.tagName.toLowerCase();
      let childrenMd = '';
      node.childNodes.forEach(child => {
        childrenMd += this.processNode(child);
      });

      switch (tag) {
        case 'h1': return `\n\n# ${childrenMd.trim()}\n\n`;
        case 'h2': return `\n\n## ${childrenMd.trim()}\n\n`;
        case 'h3': return `\n\n### ${childrenMd.trim()}\n\n`;
        case 'h4': return `\n\n#### ${childrenMd.trim()}\n\n`;
        case 'h5': return `\n\n##### ${childrenMd.trim()}\n\n`;
        case 'h6': return `\n\n###### ${childrenMd.trim()}\n\n`;
        case 'p': return `\n\n${childrenMd.trim()}\n\n`;
        case 'strong':
        case 'b': return ` **${childrenMd.trim()}** `;
        case 'em':
        case 'i': return ` *${childrenMd.trim()}* `;
        case 'code':
          if (node.parentElement && node.parentElement.tagName.toLowerCase() === 'pre') {
            return childrenMd;
          }
          return ` \`${childrenMd.trim()}\` `;
        case 'pre': {
          const lang = node.querySelector('code')?.className?.match(/language-(\w+)/)?.[1] || '';
          return `\n\n\`\`\`${lang}\n${node.textContent.trim()}\n\`\`\`\n\n`;
        }
        case 'blockquote': return `\n\n> ${childrenMd.trim().replace(/\n/g, '\n> ')}\n\n`;
        case 'ul': return `\n\n${childrenMd}\n\n`;
        case 'ol': return `\n\n${childrenMd}\n\n`;
        case 'li': {
          const isOrdered = node.parentElement && node.parentElement.tagName.toLowerCase() === 'ol';
          const prefix = isOrdered ? '1. ' : '- ';
          return `${prefix}${childrenMd.trim()}\n`;
        }
        case 'a': {
          const href = node.getAttribute('href');
          const title = childrenMd.trim() || href;
          return href ? `[${title}](${href})` : title;
        }
        case 'img': {
          const src = node.getAttribute('src') || node.getAttribute('data-src') || '';
          const alt = node.getAttribute('alt') || '图片';
          return src ? `\n\n![${alt}](${src})\n\n` : '';
        }
        case 'hr': return '\n\n---\n\n';
        case 'br': return '\n';
        default:
          return childrenMd;
      }
    }
  }

  global.MiniTurndown = MiniTurndown;
})(window);
