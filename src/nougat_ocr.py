# nougat_ocr.py
import pypdfium2 as pdfium
from nougat import NougatModel
from nougat.utils.checkpoint import get_checkpoint
from nougat.dataset.rasterize import rasterize_paper
from nougat.utils.device import move_to_device
import torch
from tqdm import tqdm

def load_nougat_model(checkpoint_path):
    model = NougatModel.from_pretrained(checkpoint_path)
    model = move_to_device(model)
    model.eval()
    return model

def pdf_to_images(pdf_path):
    pdf = pdfium.PdfDocument(pdf_path)
    images = rasterize_paper(pdf)
    return images

def extract_text_from_pdf_file(pdf_path, checkpoint_path):
    model = load_nougat_model(checkpoint_path)
    images = pdf_to_images(pdf_path)
    predictions = []

    for image in tqdm(images):
        model_input = model.encoder.prepare_input(image, random_padding=False)
        model_input = model_input.unsqueeze(0)  # Add batch dimension
        with torch.no_grad():
            output = model.inference(image_tensors=model_input)
        predictions.append(output['predictions'][0])
    
    return '\n'.join(predictions)

