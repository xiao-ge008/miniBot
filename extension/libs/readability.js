/**
 * miniBot 网页正文智能提纯引擎 (基于启发式算法)
 * 剔除广告、侧边栏、导航、页脚等噪点，抽取主文章 DOM 与元数据
 */
(function(global) {
  class MiniReadability {
    constructor(doc) {
      this.doc = doc;
    }

    parse() {
      const doc = this.doc;
      const title = this.extractTitle(doc);
      const byline = this.extractByline(doc);
      
      // 克隆 body 进行过滤分析，不破坏原网页 DOM
      const clone = doc.body.cloneNode(true);
      this.cleanNode(clone);
      
      const articleNode = this.findBestCandidate(clone);
      const contentHtml = articleNode ? articleNode.innerHTML : clone.innerHTML;
      const textContent = articleNode ? articleNode.textContent.trim() : clone.textContent.trim();

      return {
        title,
        byline,
        contentHtml,
        textContent
      };
    }

    extractTitle(doc) {
      const ogTitle = doc.querySelector('meta[property="og:title"]');
      if (ogTitle && ogTitle.content) return ogTitle.content.trim();
      const h1 = doc.querySelector('h1');
      if (h1 && h1.textContent.trim().length > 3) return h1.textContent.trim();
      return doc.title || "网页未命名文档";
    }

    extractByline(doc) {
      const author = doc.querySelector('meta[name="author"], meta[property="article:author"]');
      if (author && author.content) return author.content.trim();
      return "";
    }

    cleanNode(root) {
      // 移除绝对不可能包含文章正文的标签
      const unwantedTags = ['script', 'style', 'noscript', 'iframe', 'svg', 'nav', 'footer', 'header', 'aside'];
      unwantedTags.forEach(tag => {
        const els = root.querySelectorAll(tag);
        els.forEach(el => el.remove());
      });

      // 移除隐藏元素
      const allEls = root.querySelectorAll('*');
      allEls.forEach(el => {
        const style = el.getAttribute('style') || '';
        if (style.includes('display: none') || style.includes('visibility: hidden')) {
          el.remove();
          return;
        }
        const className = (el.className || '').toString().toLowerCase();
        const idName = (el.id || '').toLowerCase();
        // 匹配常见广告/版权/分享栏/评论关键词
        const noiseRegex = /ad-box|advert|banner|sidebar|comment|share-btn|footer|copyright|recommend|relate/i;
        if (noiseRegex.test(className) || noiseRegex.test(idName)) {
          // 如果内部包含大量段落文字，谨慎删除
          if (el.textContent.length < 200) {
            el.remove();
          }
        }
      });
    }

    findBestCandidate(root) {
      // 1. 优先寻找语义化主标签
      const article = root.querySelector('article, [role="main"], main, .article-content, .post-content, .entry-content, #article-content');
      if (article && article.textContent.trim().length > 100) {
        return article;
      }

      // 2. 启发式评分计算：段落文字最多、文本密度最高的容器
      let bestNode = null;
      let maxScore = 0;

      const candidates = root.querySelectorAll('div, section, td');
      candidates.forEach(el => {
        let score = 0;
        const paragraphs = el.querySelectorAll('p');
        score += paragraphs.length * 20;

        const textLen = el.textContent.trim().length;
        score += Math.min(Math.floor(textLen / 100), 50);

        // 惩罚项：链接文字占比过高说明是导航或聚合列表
        const links = el.querySelectorAll('a');
        let linkTextLen = 0;
        links.forEach(a => linkTextLen += a.textContent.length);
        if (textLen > 0 && (linkTextLen / textLen) > 0.5) {
          score -= 50;
        }

        if (score > maxScore) {
          maxScore = score;
          bestNode = el;
        }
      });

      return bestNode;
    }
  }

  global.MiniReadability = MiniReadability;
})(window);
