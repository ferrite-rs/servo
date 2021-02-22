use ferrite_session::*;

use cssparser::RGBA;
use euclid::default::{Point2D, Rect, Size2D, Transform2D};
use ipc_channel::ipc::{self, IpcSharedMemory};
use serde;
use serde_bytes::ByteBuf;
use style::properties::style_structs::Font as FontStyleStruct;
use std::pin::Pin;
use std::future::{Future};
use std::time::Duration;
use tokio::{task, time, runtime, sync::mpsc};
use lazy_static::lazy_static;
use crate::canvas_data::*;
use crate::canvas_paint_thread::{AntialiasMode, WebrenderApi};
use canvas_traits::canvas::*;
use gfx::font_cache_thread::FontCacheThread;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum CanvasMessage {
    Arc(Point2D<f32>, f32, f32, f32, bool),
    ArcTo(Point2D<f32>, Point2D<f32>, f32),
    DrawImage(Option<ByteBuf>, Size2D<f64>, Rect<f64>, Rect<f64>, bool),
    BeginPath,
    BezierCurveTo(Point2D<f32>, Point2D<f32>, Point2D<f32>),
    ClearRect(Rect<f32>),
    Clip,
    ClosePath,
    Ellipse(Point2D<f32>, f32, f32, f32, f32, f32, bool),
    Fill(FillOrStrokeStyle),
    FillText(String, f64, f64, Option<f64>, FillOrStrokeStyle, bool),
    FillRect(Rect<f32>, FillOrStrokeStyle),
    LineTo(Point2D<f32>),
    MoveTo(Point2D<f32>),
    QuadraticCurveTo(Point2D<f32>, Point2D<f32>),
    Rect(Rect<f32>),
    RestoreContext,
    SaveContext,
    StrokeRect(Rect<f32>, FillOrStrokeStyle),
    Stroke(FillOrStrokeStyle),
    SetLineWidth(f32),
    SetLineCap(LineCapStyle),
    SetLineJoin(LineJoinStyle),
    SetMiterLimit(f32),
    SetGlobalAlpha(f32),
    SetGlobalComposition(CompositionOrBlending),
    SetTransform(Transform2D<f32>),
    SetShadowOffsetX(f64),
    SetShadowOffsetY(f64),
    SetShadowBlur(f64),
    SetShadowColor(RGBA),
    SetFont(FontStyleStruct),
    SetTextAlign(TextAlign),
    SetTextBaseline(TextBaseline),
    PutImageData(Rect<u64>, IpcSharedMemory),
    Recreate(Size2D<u64>),
}

define_choice! { CanvasOps;
  Message: ReceiveValue <
    CanvasMessage,
    Z
  >,
  Messages: ReceiveValue <
    Vec < CanvasMessage >,
    Z
  >,
  GetTransform: SendValue<
    Transform2D<f32>,
    Z
  >,
  GetImageData: ReceiveValue <
    ( Rect<u64>, Size2D<u64>, ipc::IpcBytesSender ),
    Z
  >,
  IsPointInPath: ReceiveValue <
    ( f64, f64, FillRule ),
    SendValue <
      bool,
      Z
    >
  >,
  FromLayout: SendValue <
    Option<CanvasImageData>,
    Z
  >,
  FromScript: ReceiveValue <
    ipc::IpcSender<IpcSharedMemory>,
    Z
  >,
}

pub type CanvasSession = LinearToShared<ExternalChoice<CanvasOps>>;

pub type CreateCanvasSession =
    LinearToShared<ReceiveValue<(Size2D<u64>, bool), SendValue<SharedChannel<CanvasSession>, Z>>>;

fn handle_canvas_message(canvas: &mut CanvasData<'static>, message: CanvasMessage) {
  info!("handling CanvasMessage {:?}", message);
  match message {
    CanvasMessage::FillText(text, x, y, max_width, style, is_rtl) => {
      canvas.set_fill_style(style);
      canvas.fill_text(text, x, y, max_width, is_rtl);
    },
    CanvasMessage::FillRect(rect, style) => {
      canvas.set_fill_style(style);
      canvas.fill_rect(&rect);
    },
    CanvasMessage::StrokeRect(rect, style) => {
        canvas.set_stroke_style(style);
        canvas.stroke_rect(&rect);
    },
    CanvasMessage::ClearRect(ref rect) => {
      info!("calling clear_rect");
      canvas.clear_rect(rect);
      info!("clear_rect done");
    },
    CanvasMessage::BeginPath => canvas.begin_path(),
    CanvasMessage::ClosePath => canvas.close_path(),
    CanvasMessage::Fill(style) => {
        canvas.set_fill_style(style);
        canvas.fill();
    },
    CanvasMessage::Stroke(style) => {
        canvas.set_stroke_style(style);
        canvas.stroke();
    },
    CanvasMessage::Clip => canvas.clip(),
    CanvasMessage::DrawImage(
        imagedata,
        image_size,
        dest_rect,
        source_rect,
        smoothing_enabled,
    ) => {
        let data = imagedata.map_or_else(
            || vec![0; image_size.width as usize * image_size.height as usize * 4],
            |bytes| bytes.into_vec(),
        );
        canvas.draw_image(
            data,
            image_size,
            dest_rect,
            source_rect,
            smoothing_enabled,
        )
    },
    CanvasMessage::MoveTo(ref point) => canvas.move_to(point),
    CanvasMessage::LineTo(ref point) => canvas.line_to(point),
    CanvasMessage::Rect(ref rect) => canvas.rect(rect),
    CanvasMessage::QuadraticCurveTo(ref cp, ref pt) => {
        canvas.quadratic_curve_to(cp, pt)
    },
    CanvasMessage::BezierCurveTo(ref cp1, ref cp2, ref pt) => {
        canvas.bezier_curve_to(cp1, cp2, pt)
    },
    CanvasMessage::Arc(ref center, radius, start, end, ccw) => {
        canvas.arc(center, radius, start, end, ccw)
    },
    CanvasMessage::ArcTo(ref cp1, ref cp2, radius) => {
        canvas.arc_to(cp1, cp2, radius)
    },
    CanvasMessage::Ellipse(ref center, radius_x, radius_y, rotation, start, end, ccw) =>
      canvas
        .ellipse(center, radius_x, radius_y, rotation, start, end, ccw),
    CanvasMessage::RestoreContext => canvas.restore_context_state(),
    CanvasMessage::SaveContext => canvas.save_context_state(),
    CanvasMessage::SetLineWidth(width) => canvas.set_line_width(width),
    CanvasMessage::SetLineCap(cap) => canvas.set_line_cap(cap),
    CanvasMessage::SetLineJoin(join) => canvas.set_line_join(join),
    CanvasMessage::SetMiterLimit(limit) => canvas.set_miter_limit(limit),
    CanvasMessage::SetTransform(ref matrix) => canvas.set_transform(matrix),
    CanvasMessage::SetGlobalAlpha(alpha) => canvas.set_global_alpha(alpha),
    CanvasMessage::SetGlobalComposition(op) => {
        canvas.set_global_composition(op)
    },
    CanvasMessage::SetShadowOffsetX(value) => {
        canvas.set_shadow_offset_x(value)
    },
    CanvasMessage::SetShadowOffsetY(value) => {
        canvas.set_shadow_offset_y(value)
    },
    CanvasMessage::SetShadowBlur(value) => canvas.set_shadow_blur(value),
    CanvasMessage::SetShadowColor(color) => canvas.set_shadow_color(color),
    CanvasMessage::SetFont(font_style) => canvas.set_font(font_style),
    CanvasMessage::SetTextAlign(text_align) => {
        canvas.set_text_align(text_align)
    },
    CanvasMessage::SetTextBaseline(text_baseline) => {
        canvas.set_text_baseline(text_baseline)
    },
    CanvasMessage::PutImageData(rect, img) => {
        info!("PutImageData");
        canvas.put_image_data(img.to_vec(), rect);
    },
    CanvasMessage::Recreate(size) => {
        canvas.recreate(size);
    },
  }

  info!("done handling CanvasMessage");
}

pub fn canvas_session(mut canvas: CanvasData<'static>) -> SharedSession<CanvasSession> {
    accept_shared_session(offer_choice! {
      Message => {
        receive_value! ( message => {
          handle_canvas_message (&mut canvas, message);
          detach_shared_session (
            canvas_session ( canvas )
          )
        })
      },
      Messages => {
        receive_value! ( messages => {
          info!("handling CanvasMessages {:?}", messages);
          for message in messages {
            handle_canvas_message (&mut canvas, message);
          }

          detach_shared_session (
            canvas_session ( canvas )
          )
        })
      },
      GetTransform => {
        info!("GetTransform");
        let transform = canvas.get_transform();
        send_value! ( transform,
          detach_shared_session (
            canvas_session ( canvas )
          ))
      },
      GetImageData => {
        info!("GetImageData");
        receive_value( move | msg: ( Rect<u64>, Size2D<u64>, ipc::IpcBytesSender ) | async move {
          let (dest_rect, canvas_size, sender) = msg;
          let pixels = canvas.read_pixels(dest_rect, canvas_size);
          sender.send(&pixels).unwrap();

          detach_shared_session (
            canvas_session ( canvas )
          )
        })
      },
      IsPointInPath => {
        info!("IsPointInPath");
        receive_value!( msg => {
          let (x, y, fill_rule) = msg;
          let res = canvas.is_point_in_path_bool(x, y, fill_rule);

          send_value!(res,
            detach_shared_session (
              canvas_session ( canvas )
            ))
        })
      },
      FromLayout => {
        info!("FromLayout");
        send_value ( canvas.get_data(),
          detach_shared_session (
            canvas_session ( canvas )
          ))
      },
      FromScript => {
        info!("FromScript");
        receive_value! ( sender => {
          canvas.send_pixels(sender);

          detach_shared_session (
            canvas_session ( canvas )
          )
        })
      },
    })
}

pub struct CanvasContext {
    webrender_api: Box<dyn WebrenderApi>,
    font_cache_thread: FontCacheThread,
}

pub fn run_create_canvas_session(ctx: CanvasContext) -> SharedSession<CreateCanvasSession> {
    accept_shared_session(receive_value!( param => {
      let (size, antialias) = param;

      let antialias_mode = if antialias {
          AntialiasMode::Default
      } else {
          AntialiasMode::None
      };

      let canvas = CanvasData::new(
        size,
        ctx.webrender_api.clone(),
        antialias_mode,
        ctx.font_cache_thread.clone(),
      );

      let (session, _) = run_shared_session (
        canvas_session ( canvas )
      );

      send_value! ( session,
        detach_shared_session (
          run_create_canvas_session ( ctx )
        ) )
    } ))
}

pub fn create_canvas_session(
    webrender_api: Box<dyn WebrenderApi>,
    font_cache_thread: FontCacheThread,
) -> SharedChannel<CreateCanvasSession> {
    let ctx = CanvasContext {
        webrender_api: webrender_api,
        font_cache_thread: font_cache_thread,
    };

    let (channel, _) = run_shared_session(run_create_canvas_session(ctx));

    channel
}

// pub async fn draw_image_in_other(
//     source: SharedChannel<CanvasSession>,
//     target: SharedChannel<CanvasSession>,
//     image_size: Size2D<f64>,
//     dest_rect: Rect<f64>,
//     source_rect: Rect<f64>,
//     smoothing: bool,
// ) {
//     debug!("[draw_image_in_other] acquiring shared session");

//     run_session(acquire_shared_session!(source, source_chan =>
//     choose!(
//         source_chan,
//         GetImageData,
//         send_value_to!(
//             source_chan,
//             (source_rect.to_u64(), image_size.to_u64()),
//             receive_value_from(source_chan, move | image: IpcSharedMemory | async move {
//                 release_shared_session(
//                     source_chan,
//                     acquire_shared_session!(target, target_chan =>
//                         choose!(
//                             target_chan,
//                             Message,
//                             send_value_to!(
//                                 target_chan,
//                                 CanvasMessage::DrawImage(
//                                     Some(ByteBuf::from(image.to_vec())),
//                                     source_rect.size,
//                                     dest_rect,
//                                     source_rect,
//                                     smoothing
//                                 ),
//                                 release_shared_session(target_chan, terminate())
//                             ))))
//             }))
//                         )
//                         ))
//     .await;

//     debug!("released shared session");
// }

lazy_static! {
  pub static ref RUNTIME : runtime::Runtime =
    runtime::Builder::new_multi_thread()
      .worker_threads(16)
      .max_blocking_threads(1024)
      .enable_time()
      .build()
      .unwrap();
}

enum QueueItem {
  Yield,
  Message( CanvasMessage ),
  Task ( Pin < Box <
    dyn Future< Output=() > + Send + 'static
  > > ),
}

#[derive(Clone)]
pub struct AsyncQueue {
  task_sender:
    mpsc::UnboundedSender < QueueItem >
}

fn send_canvas_messages (
  session: SharedChannel < CanvasSession >,
  messages: Vec < CanvasMessage >,
) {
  async_acquire_shared_session ( session, move | chan | async move {
      choose! ( chan, Messages,
          send_value_to! ( chan, messages,
              release_shared_session (chan,
                  terminate! () ) ) )
  });
}

impl AsyncQueue {
    pub fn new(session: SharedChannel<CanvasSession>)
      -> AsyncQueue
    {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut messages: Vec<CanvasMessage> = vec![];

        RUNTIME.spawn(async move {
            loop {
                match receiver.recv().await {
                    Some(item) => {
                      match item {
                        QueueItem::Message(message) => {
                          messages.push(message);
                        },
                        QueueItem::Yield => {
                          if ! messages.is_empty() {
                            send_canvas_messages(
                              session.clone(),
                              messages.split_off(0)
                            );
                          }
                        },
                        QueueItem::Task(task) => {
                          if ! messages.is_empty() {
                            send_canvas_messages(
                              session.clone(),
                              messages.split_off(0)
                            );
                          }

                          task.await;
                        }
                      }
                    }
                    None => break
                }
            }
        });

        let sender2 = sender.clone();
        RUNTIME.spawn(async move {
          loop {
            time::sleep(Duration::from_millis(20)).await;
            match sender2.send(QueueItem::Yield) {
              Err(_) => break,
              _ => {}
            }
          }
        });

        AsyncQueue {
            task_sender: sender
        }
    }

    pub fn send_canvas_message (&self, message: CanvasMessage) {
      self.task_sender.send(QueueItem::Message(message)).ok().unwrap();
    }

    pub fn enqueue_task <T, Fut> (
        &self,
        task: impl FnOnce() -> Fut
             + Send + 'static
    ) -> task::JoinHandle< T >
    where
        T: Send + 'static,
        Fut: Future< Output=T > + Send + 'static
    {
        let (sender, receiver) = once_channel();
        let job = Box::pin(async move {
            let res = task().await;
            sender.send(res).unwrap();
        });
        self.task_sender.send(QueueItem::Task(job)).ok().unwrap();

        RUNTIME.spawn(async move {
            receiver.recv().await.unwrap()
        })
    }
}
