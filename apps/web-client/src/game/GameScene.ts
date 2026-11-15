import Phaser from 'phaser';
import type { EntityStateWire } from '../network/protocol.js';

export interface GameSceneCallbacks {
  onTileClick?: (worldX: number, worldY: number) => void;
  onEntityClick?: (entityId: string, entityType: string) => void;
}

export class WorldGameScene extends Phaser.Scene {
  private entitySprites = new Map<string, { container: Phaser.GameObjects.Container; hpBar: Phaser.GameObjects.Graphics }>();
  private localActorId = '';
  private callbacks: GameSceneCallbacks = {};
  private gridGraphics: Phaser.GameObjects.Graphics | null = null;
  private cursors: Phaser.Types.Input.Keyboard.CursorKeys | null = null;
  private wasdKeys: { W: Phaser.Input.Keyboard.Key; A: Phaser.Input.Keyboard.Key; S: Phaser.Input.Keyboard.Key; D: Phaser.Input.Keyboard.Key } | null = null;

  constructor() {
    super({ key: 'WorldGameScene' });
  }

  public init(data: { localActorId: string; callbacks?: GameSceneCallbacks }) {
    this.localActorId = data.localActorId;
    if (data.callbacks) {
      this.callbacks = data.callbacks;
    }
  }

  public create(): void {
    // Background Grid
    this.gridGraphics = this.add.graphics();
    this.drawGrid(2000, 2000, 32);

    // Keyboard inputs
    if (this.input.keyboard) {
      this.cursors = this.input.keyboard.createCursorKeys();
      this.wasdKeys = {
        W: this.input.keyboard.addKey(Phaser.Input.Keyboard.KeyCodes.W),
        A: this.input.keyboard.addKey(Phaser.Input.Keyboard.KeyCodes.A),
        S: this.input.keyboard.addKey(Phaser.Input.Keyboard.KeyCodes.S),
        D: this.input.keyboard.addKey(Phaser.Input.Keyboard.KeyCodes.D),
      };
    }

    // Pointer click for movement
    this.input.on('pointerdown', (pointer: Phaser.Input.Pointer) => {
      const worldX = Math.round(pointer.worldX);
      const worldY = Math.round(pointer.worldY);
      this.callbacks.onTileClick?.(worldX, worldY);
    });

    // Camera limits
    this.cameras.main.setBounds(-1000, -1000, 4000, 4000);
    this.cameras.main.setBackgroundColor('#161b22');
  }

  public update(): void {
    const camSpeed = 8;
    if (this.cursors && this.wasdKeys) {
      if (this.cursors.left.isDown || this.wasdKeys.A.isDown) {
        this.cameras.main.scrollX -= camSpeed;
      } else if (this.cursors.right.isDown || this.wasdKeys.D.isDown) {
        this.cameras.main.scrollX += camSpeed;
      }

      if (this.cursors.up.isDown || this.wasdKeys.W.isDown) {
        this.cameras.main.scrollY -= camSpeed;
      } else if (this.cursors.down.isDown || this.wasdKeys.S.isDown) {
        this.cameras.main.scrollY += camSpeed;
      }
    }
  }

  public updateEntities(entities: Map<string, EntityStateWire>, localActorId: string): void {
    this.localActorId = localActorId;
    const currentIds = new Set(entities.keys());

    // Remove sprites no longer in entity map
    for (const [id, entry] of this.entitySprites.entries()) {
      if (!currentIds.has(id)) {
        entry.container.destroy();
        this.entitySprites.delete(id);
      }
    }

    // Update or add entities
    for (const [id, ent] of entities.entries()) {
      let entry = this.entitySprites.get(id);

      if (!entry) {
        // Create container for entity (sprite circle + name tag + HP bar)
        const container = this.add.container(ent.x, ent.y);

        // Entity Body Circle
        const eType = (ent.entity_type || '').toLowerCase();
        const color =
          id === this.localActorId
            ? 0x2ea043 // Bright Green for Local Player
            : eType.includes('monster') || eType.includes('mob')
            ? 0xda3633 // Red for Monster
            : eType.includes('npc') || eType.includes('merchant')
            ? 0x388bfd // Blue for NPC
            : 0xa371f7; // Purple for other Players

        const circle = this.add.circle(0, 0, 14, color);
        circle.setStrokeStyle(2, 0xffffff, 0.9);
        circle.setInteractive({ useHandCursor: true });
        circle.on('pointerdown', (pointer: Phaser.Input.Pointer) => {
          pointer.event.stopPropagation();
          if (this.callbacks.onEntityClick) {
            this.callbacks.onEntityClick(id, ent.entity_type);
          }
        });
        container.add(circle);

        // Name text
        const nameText = this.add.text(0, -22, ent.name || id, {
          fontSize: '11px',
          color: '#ffffff',
          backgroundColor: 'rgba(0,0,0,0.6)',
          padding: { x: 3, y: 1 },
        });
        nameText.setOrigin(0.5);
        container.add(nameText);

        // HP Bar Graphics
        const hpBar = this.add.graphics();
        container.add(hpBar);

        entry = { container, hpBar };
        this.entitySprites.set(id, entry);
      }

      // Smooth interpolation or direct snap to new position
      entry.container.setPosition(ent.x, ent.y);

      // Redraw HP Bar
      entry.hpBar.clear();
      const barWidth = 28;
      const barHeight = 4;
      const barX = -barWidth / 2;
      const barY = -12;

      // Background (red)
      entry.hpBar.fillStyle(0x330000, 0.8);
      entry.hpBar.fillRect(barX, barY, barWidth, barHeight);

      // Foreground (green/yellow/red)
      const currentHp = ent.hp ?? 100;
      const maxHp = ent.max_hp ?? 100;
      const ratio = Math.max(0, Math.min(1, maxHp > 0 ? currentHp / maxHp : 1));
      const fgColor = ratio > 0.5 ? 0x2ea043 : ratio > 0.2 ? 0xd29922 : 0xcf222e;
      entry.hpBar.fillStyle(fgColor, 1.0);
      entry.hpBar.fillRect(barX, barY, barWidth * ratio, barHeight);

      // If this is the local player, center the camera
      if (id === this.localActorId) {
        this.cameras.main.pan(ent.x, ent.y, 100, 'Linear', true);
      }
    }
  }

  private drawGrid(width: number, height: number, cellSize: number): void {
    if (!this.gridGraphics) return;
    this.gridGraphics.clear();
    this.gridGraphics.lineStyle(1, 0x30363d, 0.5);

    for (let x = -width / 2; x <= width / 2; x += cellSize) {
      this.gridGraphics.moveTo(x, -height / 2);
      this.gridGraphics.lineTo(x, height / 2);
    }

    for (let y = -height / 2; y <= height / 2; y += cellSize) {
      this.gridGraphics.moveTo(-width / 2, y);
      this.gridGraphics.lineTo(width / 2, y);
    }

    this.gridGraphics.strokePath();

    // Center origin crosshair
    this.gridGraphics.lineStyle(2, 0x58a6ff, 0.8);
    this.gridGraphics.strokeCircle(0, 0, 4);
  }

  public setLocalActorId(actorId: string): void {
    this.localActorId = actorId;
  }

  public showDamageText(x: number, y: number, amount: number, isCrit: boolean = false): void {
    const text = this.add.text(x, y - 10, `-${amount}`, {
      fontSize: isCrit ? '16px' : '13px',
      color: isCrit ? '#ff4d4f' : '#ff7875',
      fontStyle: 'bold',
      stroke: '#000000',
      strokeThickness: 3,
    });
    text.setOrigin(0.5);

    this.tweens.add({
      targets: text,
      y: y - 35,
      alpha: 0,
      duration: 800,
      ease: 'Power1',
      onComplete: () => {
        text.destroy();
      },
    });
  }

  public triggerHitEffect(entityId: string): void {
    const entry = this.entitySprites.get(entityId);
    if (!entry) return;

    this.tweens.add({
      targets: entry.container,
      alpha: 0.3,
      yoyo: true,
      repeat: 1,
      duration: 60,
    });
  }
}
